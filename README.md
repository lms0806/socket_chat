# socket_communication

## 사용법

```aiignore
cargo build --release

- 서버
cargo run -- --mode server --addr 127.0.0.1:9000 --name Server

- 클라
cargo run -- --mode client --addr 127.0.0.1:9000 --name Client
```