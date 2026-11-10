# Bricks documentation

Windows host and Windows client are the first supported platforms.

| Document | Purpose |
| --- | --- |
| [Development status](development.md) | What works, how to build and test, current limits |
| [Product design](design.md) | Scope, behaviour and operating modes |
| [Architecture](architecture.md) | Components, threads and OS boundaries |
| [Design review](design-review.md) | Decisions and open requirements |
| [Frontend design](frontend-design.md) | GUI screens, pairing and feedback |
| [Slint desktop](slint-desktop.md) | Building, running and testing the desktop app |
| [Agent and network recovery](single-pc-agent-and-recovery.md) | Headless agent, QUIC recovery, fault and stress tests |
| [Local control SDK](control-sdk.md) | Named-pipe contract between the agent, GUI and CLI |
| [Protocol](protocol.md) | Delivery, event identity, recovery and session lifecycle |
| [Timing policy](timing-policy.md) | Forwarding and timestamp rules |
| [Configuration](configuration.md) | Profiles, validation and backends |
| [Identity and ownership](identity-and-ownership.md) | DPAPI credentials, peer trust and revocation |
| [QUIC probe](quic-probe.md) | Loopback and two-PC diagnostic |
| [Recovery self-test](recovery-selftest.md) | One-command fault suite |
| [Diagnostic launcher](desktop-ui.md) | Older WinForms test tool |
| [Installation and connection](installation-and-connection.md) | Setup, pairing, discovery and packaging |
| [Targets and benchmarks](targets-and-benchmarks.md) | Latency targets and how they are measured |
| [Roadmap](roadmap.md) | Milestones and open questions |
| [Tooling](tooling.md) | Language, dependencies and dev tools |
| [Sources](sources/README.md) | External references |

Test results are in [test-results](test-results/).
