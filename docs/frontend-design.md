# Bricks product frontend

Decision: 2026-10-23. User-selected direction: Slint, simple controls, no animations, functionality and clear event feedback. The [Slint desktop](slint-desktop.md) implements saved-PC management, settings, protected identity setup and checks with event feedback. It now connects to a separate per-user Rust agent through the [implemented IPC SDK subset](single-pc-agent-and-recovery.md). Live remote OS input/sharing and media remain gated. The WinForms launcher remains a separate development tool.

## Scope and appearance

One Windows application provides both Share and Connect. Use three main screens: **Computers**, **Share this PC**, and **Settings**. Pairing and incoming approval use small dialogs. Put diagnostics inside **Settings > Advanced**.

Use standard-looking buttons, a readable computer list, system fonts, a neutral background and one accent colour. Status always has text; colour is supplementary. No animations, blinking, marquee progress bars, splash sequence, decorative illustrations, promotional hero, default charts or fabricated example computers. Pending work shows a short status and a Cancel action. Keep keyboard navigation, focus visibility, selectable error text and high-DPI layouts.

## Computers: default screen

```text
Bricks                                            [Settings]

Computers                  [Add PC]  [Share this PC]
-----------------------------------------------------------
Name                 Status                       Action
Gaming PC            Saved; availability unknown   [Connect]
Laptop               Needs pairing                [Pair]
-----------------------------------------------------------
Status: Not connected. This PC is not sharing.
[Recent activity]
```

Names above illustrate layout only; a new installation has an empty list and **Add PC**. Keep the list and actions above the activity area. Show each saved computer's name and actual pairing/availability state. A saved address alone means neither paired nor online. Discovery, when implemented, supplies untrusted candidates and is active only under the network/sharing policy.

**Add PC** accepts a friendly name and a reachable LAN/tunnel address when discovery is unavailable. The port has a sensible default and is an Advanced option. Saving an address does not grant trust. **Pair** begins verified pairing; **Connect** requests an authorized session with that computer. Persist names, addresses and profile choices through the agent, rather than separate frontend-owned copies of trust/session state.

A selected device exposes Rename, Edit address, and Forget. Forget removes the saved entry; revoking trust is an explicit, separately named action. Identity changes stop connection and require verification again. Do not silently adopt a new fingerprint or label a timeout as a confirmed offline device.

## Share this PC

```text
[Back]  Share this PC

This computer: My PC
Sharing: Off                              [Start sharing]

Paired devices
Laptop                      Approval required    [Revoke]

Status: No device is controlling this PC.
```

Start sharing is an explicit agent request. Display Sharing only after the listener and admission policy are ready. The host sees incoming requests with the verified device identity and **Allow once / Decline**. A busy host reports the existing controlling device; it does not grant two owners.

During control, show **Being controlled by Laptop**, **Stop control**, and **Stop sharing**. Stop sharing first disables new admissions, then stops the current session and attempts release of owned input. Show cleanup progress or failure rather than hiding it behind an Off label. A failed release offers **Retry release**.

Paired devices still require session approval by default. Optional unattended reconnect is a later explicit permission; do not show a working toggle before it is implemented. Sharing and discovery must not start just because the app or agent launched.

## Pairing

The intended normal flow is: host creates a short-lived offer; client enters its code; verified device identities are bound to that exchange; host approves; agent persists trust; UI displays **Paired**. Show expiry as plain text. Cancellation or expiry clears the offer, and an invalid code never grants trust.

Short-code pairing requires a reviewed PAKE/identity-binding implementation with the existing expiry/attempt policies. A UI code generator alone does not implement pairing. Until that capability exists, show **Code pairing is not available in this build**. Keep the existing fingerprint-file method explicitly labelled **Development pairing** under Advanced. Do not make ordinary users handle certificate files in the intended release flow.

## Settings

Keep ordinary settings small: computer name, supported connection/input profile, and paired-device permissions. Save through the shared versioned configuration model; invalid settings leave the previous valid settings intact. Explain changes which require a new session.

Advanced holds explicit addresses/port, backend/recovery options, diagnostics and report export. The UI lists only registered, supported backends and capabilities. Do not invent arbitrary plugin names or settings the engine cannot apply. Video/audio options appear when those backends exist; there are no decorative resolution/FPS controls which do nothing. Start-at-login/background-sharing options follow an implemented lifecycle, visible tray state and installer support.

## Session surface and lifecycle

The eventual streaming viewer is a native presentation surface with a small optional toolbar: computer name, input state, fullscreen and Disconnect. Video frames and real-time input do not pass through Slint bindings or the control SDK. Slint is the setup/control interface, not a per-frame video/image uploader.

Input-only sessions are explicitly labelled **Input only** and never display a pretend video area. Hybrid mode stays an advanced, separately validated mode with one input owner. Full Bricks streaming remains gated by the media roadmap.

For the first product iteration, closing the main app requests stop/disconnect and completes cleanup before normal exit. Explicit headless operation through the CLI remains possible; it does not start automatically. Background sharing after closing the control window is a later opt-in feature requiring a visible tray indicator and local stop control. Viewer focus loss or close suspends capture and requests release regardless of control-window visibility. Abnormal termination still requires separately tested watchdog/supervisor protection.

## Event feedback

Always show a current status line. Important errors remain visible until resolved or acknowledged; acknowledging a message cannot clear the underlying fault. Recent activity contains at most 100 timestamped, plain-language control events in memory. No keystrokes, pairing codes, private keys or per-packet spam enter that list. Detailed tracing remains opt-in under Advanced.

| Agent event / condition | User feedback | Available action |
| --- | --- | --- |
| Request sent | Connecting to Gaming PC... | Cancel |
| Host approval pending | Waiting for approval on Gaming PC | Cancel |
| Pairing completed and saved | Paired with Gaming PC | Connect when supported |
| Session actually active | Connected to Gaming PC - Input only / Streaming | Disconnect |
| Host listening/admission ready | Sharing this PC | Stop sharing |
| Input capture suspended | Input paused - return to the session to resume | Resume when safe |
| Connection lost | Connection lost. Input paused on this PC | Reconnect / Disconnect |
| Remote cleanup not confirmed | Remote key release has not been confirmed | Keep status visible; inspect host |
| Checked owned releases complete | Input released | Reconnect if session policy permits |
| Release failed | Control stopped. Some keys may still be held | Retry release |
| Offer expired / approval declined | Pairing expired / Request declined | Pair again / Return |
| Identity changed | This PC's identity changed. Verify it again | Verify / Cancel |
| Agent or event channel lost | Bricks agent unavailable. Status cannot be confirmed | Retry agent connection |
| Agent restarted after an unclean exit | Previous input state cannot be confirmed | Show recovery status; block new control until safe |
| Feature unavailable | Remote input / streaming is not available in this build | Relevant setup or Advanced only |

Update important state on receipt of its event without an artificial delay. Coalesce optional aggregate statistics; they are hidden by default. A generic timeout does not prove firewall blockage or identify a direct/relayed route. Display route Unknown unless the engine has evidence.

## Capability rules and delivery order

Before enabling session actions, read the agent's capabilities and current state. A disabled action has an adjacent explanation. A diagnostic PASS is never evidence that a live input/video capability exists. Never simulate a successful session to fill an unfinished screen.

The design step is complete when these layouts and the [control SDK contract](control-sdk.md) agree. Implement next in this order: typed control contract and state reducer; agent/SDK handshake and device/settings persistence; Slint screens bound to real state; supported pairing/session adapters; then integrate input and eventual native media under their existing milestone gates. The first runnable Slint slice should let a user save/edit a PC and see capability/agent feedback. It must not make another diagnostics dashboard the product home screen.

Acceptance: one application/installer plan, no mandatory terminal/TOML in normal use, controls usable by keyboard, exact pending/success/failure/cancel feedback, bounded activity/state delivery, and no GUI work on the input path. Test agent restart, UI reconnect, duplicate clicks, delayed completion, cancellation racing success, stale events, lost authorization and release failures. Physical latency, real OS cleanup, short-code security, clean installation and first-user usability retain their separate acceptance gates.
