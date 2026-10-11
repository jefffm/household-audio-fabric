"""Finite, source-bound generated WAV retrieval; no bearer URLs or redirects."""

import re
from http.server import BaseHTTPRequestHandler


def handler(service):
    class Media(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_HEAD(self):
            self.serve_media(head=True)

        def do_GET(self):
            self.serve_media(head=False)

        def serve_media(self, head):
            match = re.fullmatch(r"/media/([a-f0-9]{32})/([A-Za-z0-9_-]{1,64})\.wav", self.path)
            if not match:
                self.send_error(404)
                return
            job, output = match.groups()
            data = service.media_data(job, output, self.client_address[0], head)
            if data is None:
                self.send_error(404)
                return
            size = len(data)
            start, end = 0, size - 1
            requested = self.headers.get("Range")
            if requested:
                byte_range = re.fullmatch(r"bytes=([0-9]*)-([0-9]*)", requested)
                try:
                    if not byte_range or not any(byte_range.groups()):
                        raise ValueError()
                    left, right = byte_range.groups()
                    if left:
                        start = int(left)
                        end = min(size - 1, int(right)) if right else size - 1
                    else:
                        start = max(0, size - int(right))
                    if not 0 <= start <= end < size:
                        raise ValueError()
                except ValueError:
                    self.send_response(416)
                    self.send_header("Content-Range", f"bytes */{size}")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
            self.send_response(206 if requested else 200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            if requested:
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.end_headers()
            if not head:
                self.wfile.write(data[start:end + 1])
                self.wfile.flush()
                if start == 0 and end == size - 1:
                    service.fetched(job, output)

    return Media
