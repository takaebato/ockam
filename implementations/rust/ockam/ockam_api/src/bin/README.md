# Ingest traces

This binary ingest trace files from S3 and extract users activity for later querying.

For now this is only a prototype of what the processing pipeline could be.
The idea is to:

1. Get trace files from S3
2. Parse the Opentelemetry data
3. Extract only the user data from the spans
4. Group them by journey id
5. Store them in a database for later querying

# Current state

The current executable:

1. Reads trace files from S3 or in a local directory.
2. Deserialize spans into more compact data structures than the Opentelemetry data types.
3. Filters the spans to only keep user commands
4. Groups spans per project id
5. Prints results to the console

Processing time: it takes approximately 50 seconds to process 1 minute worth of traces retrieved from S3.

Output example:

User journey 1bf3bb53e68df4f67bd01f4ca3241030:

User name:      Eric Torreborre
User email:     etorreborre@yahoo.com
Ockam home:     /Users/etorreborre/.ockam-5
Ockam version:  0.83.0
Ockam git hash: 5d45fcc1ff55c80a163b44292278281f4bd518a5
Ockam dev.:     false
Space name:     groovy-lumpsucker
Space id:       12618bd8-069b-4aab-95d9-b3470e928d4d
Project name:   default
Project id:     bf3bb53e-68df-4f67-bd01-f4ca386da8f8

[2024-10-30 09:54:17] ✅ enroll
Execution trace id: cbd00774108f6bdb021dd661e1f9b318

[2024-10-30 09:54:17] ✅ enrolled
Execution trace id: cbd00774108f6bdb021dd661e1f9b318

[2024-10-30 09:54:23] ✅ node create
Command: ockam node create n1
Execution trace id: 8b938379729a1342f894f6dc9be27429

[2024-10-30 09:54:24] ✅ node create
Command: /Users/etorreborre/projects/ockam/ockam-5/target/debug/ockam -vv node create --tcp-listener-address 127.0.0.1:0
--foreground --child-process --opentelemetry-context {"traceparent":"
00-8b938379729a1342f894f6dc9be27429-80bb78757c286413-01","tracestate":""} n1
Node: n1
Execution trace id: 5850670af91c5f41cbef3af6f6c3efad

[2024-10-30 09:54:24] ✅ node created
Execution trace id: 8b938379729a1342f894f6dc9be27429

[2024-10-30 09:54:28] ✅ node create
Command: ockam node create n1
Execution trace id: f1d01f700ad976a9a57617ec8b7c9aa4

[2024-10-30 09:54:28] ❌ node create error
Command: ockam node create n1
Error: Node n1 has been already created
Execution trace id: 2a18fda69ce2d01472731c5b96cc6fde

[2024-10-30 09:54:32] ✅ node list
Command: ockam node list
Execution trace id: 216f567e32b20a7024fdc11c684ed434

[2024-10-30 09:54:38] ✅ project list
Command: ockam project list
Execution trace id: f3d703a630df21fb698b35504cb84a1d
