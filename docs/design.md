# Product design

## Objective

Build Bricks into a responsive remote gaming system that faithfully transports player input and minimizes physical input-to-photon latency. The competitive ambition is to beat a tuned Sunshine + Moonlight setup on a declared hardware/network profile while retaining comparable image quality, frame stability, security, and compatibility.

The first deliverable is an input transport with a benchmark harness. The later media path is necessary before claiming a complete alternative.

## Network independence

Bricks runs over ordinary IP connectivity: direct LAN, Tailscale, WireGuard, or another IP tunnel that carries the selected backend's traffic. No tunnel-specific API or account is required for connection; an explicit reachable host address always remains available. The initial QUIC backend requires UDP reachability between the peers. Local discovery is optional and may not cross tunnel/subnet boundaries. Bricks implements no custom NAT traversal, rendezvous service, or relay in v1; any tunnel traversal or relay behavior belongs to the external network provider.

## Modes

| Mode | What it does | What it can establish |
| --- | --- | --- |
| Local harness | Capture, decode, state machine, and injection without a network | Correctness and software overhead |
| Input only | Bricks capture and injection across a network | Input-path timing, recovery, and game compatibility |
| Hybrid | Bricks input beside Sunshine video and Moonlight viewing | Whether changing only input improves the visible result |
| Full Bricks | Bricks input, video, audio, decode, and presentation | Whole-system comparison against Sunshine + Moonlight |

Hybrid mode requires exactly one remote input owner. Disable the comparison stack's keyboard/mouse forwarding and verify this with a counter fixture. If the chosen version cannot disable input reliably, add an explicit test adapter or mark hybrid testing unavailable. Running both forwarders invalidates results.

## Live input behavior

Capture physical key transitions and raw relative mouse deltas. Timestamp at application capture, forward without deliberate delay, and inject promptly. Never sample release delays or alter the user's timing to fit a distribution.

Negotiate explicit physical key identifiers and map them per OS. Preserve left/right modifiers, extended keys, wheel units, and button identity. Host layout determines resulting text; physical-key forwarding is not an IME or text transport. Distinguish physical transitions from OS-generated repeats. Forward a held state once and let the host own repeat behavior; test repeat separately.

Gameplay relative motion and desktop absolute pointing are separate negotiated capabilities. No default acceleration correction; validate effective camera sensitivity and document client/host settings. Optional aggregation sums deltas only between ordering barriers. Do not combine across clicks, wheel events, key transitions, or ownership changes.

The client offers explicit capture on/off and a configurable local escape chord. Background global capture is off until activated. The host visibly indicates the controlling peer. Client focus loss releases remote state and stops forwarding until explicitly resumed. A new session begins from a neutral state; keys already held must be released locally before they can be forwarded as new presses.

On disconnect, expiry, or injection failure, release only state owned by Bricks and end or suspend control. Local host input may conflict with injected state; exclusive control is the initial supported policy, with a host-local stop action. Do not promise isolated per-user keyboards from SendInput.

## Initial scope and exclusions

Initial scope is one authorized client controlling one interactive host session, keyboard and mouse, Windows first. LAN and existing IP tunnels are supported paths. No custom NAT traversal, public relay service, clipboard, file transfer, multi-user access, controller emulation, kernel driver, secure-desktop control, or macOS implementation in the first release.

Windows-to-Windows is the confirmed first implementation. Use one application with “Share this PC” and “Connect” roles, useful defaults, and no required TOML editing. Discovery and pairing simplify LAN use; existing tunnels provide the initial remote-network route. Advanced settings remain modular and versioned as specified in [configuration](configuration.md). See [installation and connection](installation-and-connection.md) for the intended setup experience.

Launch inside the host's interactive user session. A Windows service alone is not the interactive input agent. Linux follows with constrained input-device permissions. Unsupported games remain explicit compatibility limitations; acceptance of synthetic input or anti-cheat-controlled games is never assumed.

## Expected results

The most plausible early benefit is lower input queueing and more predictable recovery. It may be small if the baseline is already efficient. A hybrid can reveal that benefit without building a codec stack. It cannot eliminate game tick latency, video encode/decode delay, network propagation, or display scanout.

The full-system opportunity is reducing unnecessary frame queues and GPU copies, preserving input priority during video congestion, and controlling presentation. No speedup percentage is promised. If careful experiments find no advantage, retain the useful input agent and publish the measured limits before committing to a larger media implementation.

See [targets and benchmarks](targets-and-benchmarks.md) for falsifiable goals and [roadmap](roadmap.md) for decisions that depend on hardware evidence.
