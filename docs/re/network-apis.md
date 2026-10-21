# Winsock APIs of the 0.5.5e client

Which calls `PathOfExileSteam.exe` (build 0.5.5e, image base `0x140000000`) uses to open network connections. This decides what the DLL's network guard has to hook ([../architecture/client-dll.md](../architecture/client-dll.md#network-guard-connect-apis), audit S-10). Static analysis plus a runtime log; nothing was run against official servers.

## Result
**The game connects with `connect` only.** No `WSAConnect`, `ConnectEx`, `WSAConnectByName` or `WSAConnectByList`.

## How it was checked
- The PE import table (read with `pefile`, then the call sites found by scanning code for `call/jmp [rip+disp]` through each import slot) lists these `WS2_32.dll` imports, all by name: `socket`, `connect`, `bind`, `listen`, `accept`, `send`, `recv`, `sendto`, `recvfrom`, `WSARecv`, `WSAIoctl`, `ioctlsocket`, `setsockopt`, `getsockopt`, `getsockname`, `getpeername`, `select`, `shutdown`, `closesocket`, `WSAEventSelect`, `WSAEnumNetworkEvents`, `WSACreateEvent`/`WSACloseEvent`/`WSAResetEvent`/`WSAWaitForMultipleEvents`, `WSAStartup`/`WSACleanup`, `WSAGetLastError`/`WSASetLastError`, `getaddrinfo`/`freeaddrinfo`, `gethostname`, `inet_addr`/`inet_ntoa`, `htons`/`htonl`/`ntohs`/`ntohl`, `__WSAFDIsSet`. No `WSAConnect*`, no `WSASend`, no `WSASocket`.
- No `mswsock.dll`, `winhttp.dll` or `wininet.dll` import (the other imported DLLs are `IPHLPAPI` for ICMP ping, `CRYPT32`, `WLDAP32` and system libraries). The only `wininet` text is a wide string in a list of module names (`wininet`, `ws2_32`, `winmm`, ...) at file offset `0x3157080`, not a load call by name that was found.
- The `ConnectEx` function GUID {25a207b9-ddf3-4660-8ee9-76e58c74063e}, the `AcceptEx` and `GetAcceptExSockaddrs` GUIDs and the strings `ConnectEx`, `WSAConnect*`, `mswsock` do not occur anywhere in the file (searched as bytes and as an immediate in code).
- Runtime: with the guard hooking all of these APIs (2026-10-21, `scripts/test-run.sh` on our private server), every connection in `GameFiles/poe-hook.log` has `api=connect`, the guard never logged `ConnectEx ... hooked`, and the blocked official-service attempts (shop and telemetry hosts, also the IPv6 family as `0.0.0.0:0`) are `blocked connect`.

## `connect` (`ws2_32!connect`, import slot `0x142e89f08`)
Five call sites: `0x142198cf3`, `0x142715bc8`, `0x142b57e36`, `0x142b74444`, `0x142b74900`. The ones seen at runtime (return addresses in the log):

| Site | Used for |
|---|---|
| `0x142715bc8` | the game's login and instance TCP connections (`127.0.0.1:20481`, `127.0.0.1:6112`) |
| `0x142b57e36` | local helper connections (loopback ephemeral ports) |
| `0x142b74900` | the connection attempts to external hosts (refused in private mode: `23.220.x.x:443` and addresses of a non-IPv4 family); the purpose of the service behind it was not analysed |

The sites `0x142198cf3` and `0x142b74444` did not run in the test session.

## `WSAIoctl` (import slot `0x142e89f58`)
Two call sites, neither asks for an extension function:

| Site | Control code | Meaning |
|---|---|---|
| `0x142b4b137` | `0x4004747b` (followed by `setsockopt(SOL_SOCKET, 0x1001)` on success) | a control code not identified further; it is not `SIO_GET_EXTENSION_FUNCTION_POINTER` (`0xC8000006`) and no GUID is passed |
| `0x142b75e5b` | `0x98000004` | `SIO_KEEPALIVE_VALS` (struct of 12 bytes) |

## Other ways a socket could be opened
`GetProcAddress` and `LoadLibrary` are imported, so code could resolve any API at run time; that is why the guard also hooks `WSAConnectByName*`, `WSAConnectByList` and `ConnectEx` (through `WSAIoctl`) even though 0.5.5e uses none of them. Not covered: raw `NtDeviceIoControlFile`/AFD sockets. Under Proton `ws2_32` does not export `WSAConnectByList` (the guard logs a warning and skips it).

## Re-checking after a client update
Run the import scan again (`pefile`: list `WS2_32.dll`, `MSWSOCK.dll`, `WINHTTP.dll`, `WININET.dll` imports; scan code for the `ConnectEx` GUID bytes `b9 07 a2 25 f3 dd 60 46 8e e9 76 e5 8c 74 06 3e`); a new import or the GUID appearing means the guard's list should be re-read. The guard itself covers them already; the check only keeps this note true.
