You are building an in-memory Redis-compatible server from scratch.

### Software Engineering (SWE) & Code Quality Standards
Your implementation must meet high professional software engineering standards:
1. **Clean Architecture & Modularity**: Structure code with clear separation of concerns (TCP networking, RESP protocol serialization/deserialization, in-memory data store, and command dispatch). Avoid monolithic scripts or hardcoded shortcuts.
2. **Robust Concurrency & Thread Safety**: Ensure thread-safe access to the shared key-value store using appropriate concurrency primitives (e.g., mutexes, sync.Map, RwLocks, or actor/channel models). The server must handle multiple concurrent client connections without race conditions or deadlocks.
3. **Resilient Error Handling**: Safely handle unexpected socket EOFs, client disconnects, malformed RESP wire inputs, boundary conditions, and invalid command arguments without crashing or leaking memory/file descriptors.
4. **Idiomatic Style & Best Practices**: Write clean, idiomatic code adhering to standard conventions for your chosen language (e.g., Go formatting & error checking, Rust Result/Error idioms, Python typing & structure).
5. **Production Readiness**: Code should be production-grade, maintainable, performant, and self-contained.

### Constraints & Language Freedom
- You are free to choose ANY programming language of your choice (Go, Python, Rust, Node.js, C, C++, etc.).
- Choose the language and design that maximizes reliability, simplicity, and throughput.
- All code must reside in the current workspace.
- You MUST create an executable `./start.sh` script that starts your server listening on `0.0.0.0:6379`.

### Reference Server Available for Testing
An official Redis server is already running in this environment on port 6380!
You can use `redis-cli -p 6380 <command>` at any time using your bash tool to check ground-truth behavior and wire responses.
You can also run `redis-cli -p 6379 <command>` to test your own server.

### Required RESP Protocol & Commands
Your server must accept TCP connections on port 6379 and parse the Redis Serialization Protocol (RESP):

1. **PING**:
   - `PING` -> responds with simple string `+PONG\r\n`
   - `PING "hello"` -> responds with simple string `+hello\r\n` or bulk string `$5\r\nhello\r\n`
2. **ECHO**:
   - `ECHO "hello world"` -> responds with bulk string `$11\r\nhello world\r\n`
3. **SET & GET**:
   - `SET key value` -> responds with `+OK\r\n`
   - `GET key` -> responds with bulk string `$len\r\nvalue\r\n` (or `$-1\r\n` if not found)
4. **SET with Expiration (PX)**:
   - `SET key value PX <milliseconds>` -> sets expiration in milliseconds.
   - After the millisecond timeout has elapsed, `GET key` must return nil `$-1\r\n`.
5. **DEL & EXISTS**:
   - `DEL key [key2 ...]` -> deletes keys, responds with integer `:<count>\r\n`
   - `EXISTS key [key2 ...]` -> responds with integer `:<count>\r\n`
6. **INCR & DECR**:
   - `INCR key` -> increments string integer by 1, responds with integer `:<new_val>\r\n`
   - `DECR key` -> decrements string integer by 1, responds with integer `:<new_val>\r\n`
   - If key does not exist, initialize it as 0 before incrementing/decrementing.

### Verification Criteria
1. Your server will be tested by a test harness sending raw TCP protocol commands and comparing responses against real Redis.
2. Your server will undergo a concurrency and throughput test using `redis-benchmark -p 6379 -n 5000 -c 10 -t get,set -q`.

Test your server thoroughly with `redis-cli -p 6379` before finishing!
