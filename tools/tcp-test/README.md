This tool is useful to test the portals capabilities.

# Actions

## echo
It starts a TCP server that listens on a given port and echoes back any data it receives.
It's used in conjuction with the other actions.

# latency
Measures the latency of a connection to a given host and port.

Example:
```
$ tcp-test echo 1234
$ tcp-test latency 127.0.0.1:1234
RTT Latency: 0ms 136us
RTT Latency: 0ms 139us
RTT Latency: 0ms 109us
RTT Latency: 0ms 107us
RTT Latency: 0ms 108us
RTT Latency: 0ms 118us
RTT Latency: 0ms 106us
RTT Latency: 0ms 109us
RTT Latency: 0ms 92us
RTT Latency: 0ms 92us
Average RTT Latency: 0ms 112us
```

# flood
Floods a given host and port with connections until it fails or reaches the maximum.

Example:
```
$ tcp-test echo 1234
$ tcp-test flood 127.0.0.1:1234
Failed to connect: Os { code: 49, kind: AddrNotAvailable, message: "Can't assign requested address" }
Flooded 16344 connections
Pinging each...
All connections succeeded
```


