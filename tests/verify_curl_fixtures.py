import json
import shlex
import subprocess
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, urlsplit


class CaptureHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def capture(self):
        self.server.captured = (
            self.command,
            self.path,
            self.rfile.read(int(self.headers.get("Content-Length", 0))).decode(),
            self.headers.get("Content-Type"),
        )
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = capture


class CurlFixtureOracle(unittest.TestCase):
    def test_supported_fixtures_against_curl(self):
        cases = json.loads((Path(__file__).parent / "fixtures/import/curl.json").read_text())
        for case in cases:
            if "error" in case:
                continue
            with self.subTest(case=case["name"]):
                with HTTPServer(("127.0.0.1", 0), CaptureHandler) as server:
                    server.timeout = 4
                    worker = threading.Thread(target=server.handle_request)
                    worker.start()
                    base = f"http://127.0.0.1:{server.server_port}"
                    args = [arg.replace("https://example.test", base) for arg in shlex.split(case["command"])]
                    # Older curl versions require separate long-option values.
                    args = [part for arg in args for part in (arg.split("=", 1) if arg.startswith("--") else [arg])]
                    result = subprocess.run(
                        args + ["--noproxy", "*", "--max-time", "3", "--silent", "--show-error"],
                        capture_output=True, text=True, timeout=5,
                    )
                    worker.join(timeout=5)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    method, path, body, content_type = server.captured
                expected = urlsplit(case.get("url", "https://example.test"))
                actual = urlsplit(path)
                self.assertEqual(method, case["method"])
                self.assertEqual(body, case.get("body") or "")
                self.assertEqual(actual.path, expected.path or "/")
                self.assertEqual(
                    parse_qsl(actual.query, keep_blank_values=True),
                    parse_qsl(expected.query, keep_blank_values=True),
                )
                if case.get("body") is not None or "content_type" in case:
                    self.assertEqual(content_type, case.get("content_type", "application/x-www-form-urlencoded"))


if __name__ == "__main__":
    unittest.main()
