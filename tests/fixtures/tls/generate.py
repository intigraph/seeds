#!/usr/bin/env python3
"""Create disposable localhost certificates; never retain private keys in Git."""
from pathlib import Path
import subprocess
import sys


def generate(directory):
    directory = Path(directory)
    def run(*args):
        subprocess.run(['openssl', *args], cwd=directory, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for name in ('ca', 'foreign-ca'):
        run('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2',
            '-keyout', name + '.key', '-out', name + '.pem', '-subj', '/CN=Synthetic ' + name)
    run('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key',
        '-out', 'server.csr', '-subj', '/CN=localhost')
    (directory / 'extensions').write_text('subjectAltName=DNS:localhost\n'
        'basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n'
        'extendedKeyUsage=serverAuth\n')
    run('x509', '-req', '-in', 'server.csr', '-CA', 'ca.pem', '-CAkey', 'ca.key',
        '-CAcreateserial', '-out', 'server.pem', '-days', '2', '-extfile', 'extensions')


if __name__ == '__main__':
    generate(sys.argv[1])
