#!/usr/bin/env python3
"""Fetch a pinned CI archive, mirror first; never execute or extract it."""
import argparse
import hashlib
from http.client import IncompleteRead
import os
from pathlib import Path
import re
import shutil
import tempfile
from urllib.error import HTTPError, URLError
from urllib.parse import unquote, urlsplit
from urllib.request import HTTPRedirectHandler, build_opener

MAX_BYTES = 100 * 1024 * 1024


class HTTPSRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        parts = urlsplit(url)
        if parts.scheme != "https" or parts.username or parts.password:
            raise ValueError("unsafe asset redirect; fallback refused")
        return super().redirect_request(request, response, code, message, headers, url)


def safe_url(url):
    parts = urlsplit(url)
    if (parts.scheme != "https" or not parts.hostname or parts.username
            or parts.password or parts.query or parts.fragment):
        raise ValueError("asset URLs must be HTTPS without credentials, query or fragment")
    return url


def fetch(url, pin, destination, mirror="", *, cache=None, opener=None):
    safe_url(url)
    if not re.fullmatch(r"[0-9a-f]{64}", pin):
        raise ValueError("expected a lowercase SHA256 pin")
    name = unquote(urlsplit(url).path.rsplit("/", 1)[-1])
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name):
        raise ValueError("asset filename is not a simple basename")
    sources = []
    if mirror:
        safe_url(mirror)
        sources.append(("mirror", mirror.rstrip("/") + "/sha256/" + pin + "/" + name))
    cached = Path(cache) / "sha256" / pin / name if cache else None
    if cached is not None:
        sources.append(("cache", cached))
    sources.append(("upstream", url))
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    opener = opener or build_opener(HTTPSRedirects()).open
    for label, source in sources:
        temporary = None
        try:
            if label == "cache" and not source.exists():
                continue
            with (source.open("rb") if label == "cache" else opener(source, timeout=60)) as response:
                digest = hashlib.sha256()
                size = 0
                with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
                    temporary = Path(output.name)
                    while chunk := response.read(min(1024 * 1024, MAX_BYTES + 1 - size)):
                        size += len(chunk)
                        if size > MAX_BYTES:
                            raise ValueError("asset exceeds byte budget")
                        digest.update(chunk)
                        output.write(chunk)
                if digest.hexdigest() != pin:
                    raise ValueError("asset SHA256 mismatch; fallback refused")
                if cached is not None and label != "cache":
                    cached.parent.mkdir(parents=True, exist_ok=True)
                    with tempfile.NamedTemporaryFile(dir=cached.parent, delete=False) as saved:
                        saved_path = Path(saved.name)
                    try:
                        shutil.copyfile(temporary, saved_path)
                        os.replace(saved_path, cached)
                    finally:
                        saved_path.unlink(missing_ok=True)
                os.replace(temporary, destination)
                temporary = None
                return {"source": label, "sha256": pin, "bytes": size}
        except HTTPError as error:
            if error.code in (401, 403):
                raise ValueError("asset access refused; fallback refused") from None
            if label != "mirror":
                raise ValueError("upstream asset unavailable (HTTP %s)" % error.code) from None
        except (URLError, TimeoutError, ConnectionError, IncompleteRead):
            if label != "mirror":
                raise ValueError("upstream asset transport failed") from None
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    raise ValueError("no verified asset source")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--mirror-base", default=os.environ.get("CI_ARTIFACT_MIRROR_URL", ""))
    parser.add_argument("--cache-dir")
    args = parser.parse_args()
    try:
        receipt = fetch(args.url, args.sha256, args.output, args.mirror_base, cache=args.cache_dir)
    except ValueError as error:
        parser.exit(1, str(error) + "\n")
    except OSError:
        parser.exit(1, "cannot store verified asset\n")
    print("verified %(source)s asset: %(bytes)s bytes SHA256 %(sha256)s" % receipt)


if __name__ == "__main__":
    main()
