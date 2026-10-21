"""로컬 OpenAI 호환 /embeddings 서버 (Lantern 의미 검색을 비교 대상과 같은 모델로 재기 위해).

    python eval/bilingual/embed_server.py [--port 8711] [--model intfloat/multilingual-e5-small]

    export LANTERN_EMBED_URL=http://127.0.0.1:8711/v1
    export LANTERN_EMBED_MODEL=intfloat/multilingual-e5-small
    export LANTERN_EMBED_QUERY="query: {query}"
    export LANTERN_EMBED_DOC="passage: {path}
{text}"

baselines.py의 dense와 같은 모델·같은 접두어(query:/passage:)를 쓴다. 127.0.0.1에만 열린다.
"""

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8711)
    ap.add_argument("--model", default="intfloat/multilingual-e5-small")
    a = ap.parse_args()
    from sentence_transformers import SentenceTransformer

    from baselines import REVISION

    model = SentenceTransformer(a.model, device="cpu", revision=REVISION.get(a.model))

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            if not self.path.rstrip("/").endswith("/embeddings"):
                self.send_error(404)
                return
            body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))))
            texts = body["input"] if isinstance(body["input"], list) else [body["input"]]
            vecs = model.encode(texts, batch_size=64, normalize_embeddings=True)
            out = json.dumps({"object": "list", "model": a.model, "data": [{"object": "embedding", "index": i, "embedding": v.tolist()} for i, v in enumerate(vecs)]}).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(out)))
            self.end_headers()
            self.wfile.write(out)

        def log_message(self, *args):
            pass

    print(f"http://127.0.0.1:{a.port}/v1/embeddings ({a.model})", flush=True)
    ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
