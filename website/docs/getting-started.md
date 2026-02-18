# Getting Started

## 1) Clone And Run Server

```bash
git clone https://github.com/jaeyoung0509/Openportio.git
cd Openportio
cargo run -p openportio-server
```

Server default: `127.0.0.1:3000`

## 2) Check REST

```bash
curl -s http://127.0.0.1:3000/health
curl -s http://127.0.0.1:3000/hello/Rust
```

## 3) Check gRPC

```bash
grpcurl -plaintext 127.0.0.1:3000 list
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:3000 \
  openportio.v1.Greeter/SayHello
```

## 4) Open Docs Endpoints

- Swagger UI: `http://127.0.0.1:3000/docs`
- OpenAPI JSON: `http://127.0.0.1:3000/openapi.json`
- gRPC contracts: `http://127.0.0.1:3000/grpc/contracts`

## 5) Docs Site (This Portal)

```bash
cd website
npm install
npm run docs:dev
```

Site default: `http://127.0.0.1:5173`

## Deep References

- [`README.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/README.md)
- [`examples/simple-server/README.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/simple-server/README.md)
- [`gRPC FastAPI-like DX`](/guides/grpc-fastapi-dx)
