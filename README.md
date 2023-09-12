# ya-relay e2e test

Spawns 2 clients, ping one from the other.


## Requirements

`http_client` from [ya-relay](https://github.com/golemfactory/ya-relay/) repo. Build with `cargo build --example http_client --release`. This e2e test expects it in current directory.


## Configuration

- `RUST_LOG` - `info` by default. Set to `debug` to see `http_client` output.

Env variables are being implicitly passed to child `http_client`. Those are required:

- `API_PORT` - Port number for local http server. For second client this value is set to +1.
- `RELAY_ADDR` - Address of relay server. Like `udp://<ip.addr>:<port>`. Domain names are not supported.
