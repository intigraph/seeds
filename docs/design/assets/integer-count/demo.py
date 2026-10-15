import tempfile,subprocess,socket,time,urllib.request,urllib.parse,json,os,argparse
a=argparse.ArgumentParser();a.add_argument("--server",default="quipu-server");args=a.parse_args()
values=['0','1','9223372036854775807','0.5','-1','"0"^^<http://www.w3.org/2001/XMLSchema#decimal>','"1"^^<http://www.w3.org/2001/XMLSchema#long>','"9223372036854775808"^^<http://www.w3.org/2001/XMLSchema#integer>','"18446744073709551616"^^<http://www.w3.org/2001/XMLSchema#integer>','"1.5"^^<http://www.w3.org/2001/XMLSchema#integer>','"0"@en','"0"','<https://example.org/index>']
with tempfile.TemporaryDirectory(prefix='lta-private-') as d:
 s=socket.socket();s.bind(('127.0.0.1',0));port=s.getsockname()[1];s.close();base=f'http://127.0.0.1:{port}'
 p=subprocess.Popen([args.server,'--db',d+'/q.db','--bind',f'127.0.0.1:{port}'],cwd=d,env={**os.environ,'HOME':d},stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 try:
  for _ in range(100):
   try:urllib.request.urlopen(base+'/health',timeout=1);break
   except OSError:time.sleep(.1)
  def call(path,data,kind):
   req=urllib.request.Request(base+path,data=data,headers={'Content-Type':kind,'X-Quipu-Client':'agent-adhoc'});return urllib.request.urlopen(req,timeout=10).read()
  update='INSERT DATA { GRAPH <https://example.org/g> {'+' '.join(f'<https://example.org/n{i}> <https://example.org/p> {v} .' for i,v in enumerate(values))+'} }'
  call('/update',urllib.parse.urlencode({'update':update}).encode(),'application/x-www-form-urlencoded')
  guard='sameTerm(?v - ?v, 0) && sameTerm(?v, ?v + 0) && ?v >= 0'
  for label, condition, expected in [('before',guard,6),('after',guard+' && ?v <= 9223372036854775807',4)]:
   q='SELECT ?s ?v (STR(?v) AS ?lex) WHERE { GRAPH <https://example.org/g> { ?s <https://example.org/p> ?v FILTER('+condition+') } }'
   result=json.loads(call('/query',json.dumps({'query':q}).encode(),'application/json'))
   rows=result['rows']; print(label, 'accepted indices:', sorted(row['lex'] for row in rows), flush=True)
   assert len(rows)==expected and not result['truncated'],result
   assert any(row['lex']=='9223372036854775807' for row in rows)
  print('PASS: oversized indices excluded; maximum valid i64 preserved. Synthetic query fixture only, not installed SDK proof.',flush=True)

 finally:p.terminate();p.wait(timeout=10)
