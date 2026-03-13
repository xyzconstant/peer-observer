# `ipc` extractor

> publishes data fetched from IPC

The peer-observer ipc-extractor periodically queries data from the Bitcoin Core IPC interface and publishes the results as events into a NATS pub-sub queue

## Usage

```
$ cargo run --bin ipc-extractor -- --help
The peer-observer ipc-extractor periodically queries data from the Bitcoin Core IPC endpoint and publishes the results as events into a NATS pub-sub queue

Usage: ipc-extractor [OPTIONS] --ipc-socket-path <IPC_SOCKET_PATH>

Options:
  -i, --ipc-socket-path <IPC_SOCKET_PATH>
          A path to an UNIX socket to read IPC data from
  -a, --nats-address <ADDRESS>
          The NATS server address the extractor/tool should connect and subscribe to [default: 127.0.0.1:4222]
  -u, --nats-username <USERNAME>
          The NATS username the extractor/tool should try to authentificate to the NATS server with
  -p, --nats-password <PASSWORD>
          The NATS password the extractor/tool should try to authentificate to the NATS server with
  -f, --nats-password-file <PASSWORD_FILE>
          A path to a file containing a password the extractor/tool should try to authentificate to the NATS server with
  -l, --log-level <LOG_LEVEL>
          The log level the extractor should run with. Valid log levels are "trace", "debug", "info", "warn", "error". See https://docs.rs/log/latest/log/enum.Level.html [default: DEBUG]
      --query-interval <QUERY_INTERVAL>
          Interval (in seconds) in which to query from the Bitcoin Core IPC interface [default: 10]
  -h, --help
          Print help
  -V, --version
          Print version
```
