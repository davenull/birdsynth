# Link: playing several birdsynths in step

Link shares one transport among birdsynths: whether it's playing, the tempo,
and which beat falls when. So clips, arpeggios and tempo-synced LFOs line up
across them. Any of them can press play or stop, or turn the tempo knob, and
every one of them changes at the same moment.

- **Tabs** of one browser link by themselves, over a BroadcastChannel.
- **Computers** on the same network link with **Network** on. The site's link
  service introduces them, and then they talk directly over WebRTC. Computers
  giving the same **group code** link from any network.

Link is off until it's switched on, and so is Network, in each browser.

## The pieces

| File | What it does |
| --- | --- |
| `web/src/sync/timeline.ts` | The shared timeline `{playing, bpm, at, beat, version, keeper}`, and how play, stop and tempo change it. |
| `web/src/sync/group.ts` | The group: its members, the timekeeper, requests, heartbeats, handover. It runs over any channel. |
| `web/src/sync/channel.ts` | One channel over both transports (tabs and network): messages are numbered and duplicates dropped. |
| `web/src/sync/net.ts` | The network side: the WebSocket to the service, one WebRTC connection per peer, the relay fallback, pings. |
| `web/src/sync/clock.ts` | Each peer's clock offset, estimated from ping round trips. |
| `web/src/sync/link.ts` | Ties it together for the synth: follows the timeline in the engine, runs the drift check and the nudge, and keeps the settings. |
| `server/relay.ts`, `server/protocol.ts` | The link service: introductions, WebRTC signalling, and relaying messages. |
| `crates/engine/src/seq.rs` (`set_timeline`) | Puts the engine's song clock on a beat at an exact frame. |

## How it keeps time

**One timekeeper.** One member keeps time. Everyone else's play, stop and
tempo go to it as requests, and it applies them one at a time. Each result is
a new version of the timeline, with its moment `LEAD_MS` (150 ms) ahead, so
every member hears of it before it happens. A request that isn't answered by
a new version is sent again every 400 ms, to whoever keeps time by then.

**Who keeps time.**
- The first member keeps time, and goes on keeping it while it's there.
- When the keeper leaves, the oldest member left takes over. It carries the
  timeline on as it stands, as a new version, so nothing is heard to change.
  The keeper can leave by saying goodbye (closing or reloading its page) or by
  going silent for 3 s (a sleeping laptop).
- A newcomer listens for 600 ms before it may keep time. Over the network, it
  also waits until the service has introduced the others.
- A newcomer never takes over from a live keeper, whatever its clock says.

**One outcome everywhere.**
- Where two timelines meet (two groups joining, or a partition healing), the
  higher version wins, and ties go to the lower keeper id.
- Every heartbeat (every 500 ms) carries the sender's timeline, so a member
  that missed a change catches up within one.

**Clocks.**
- Each instance's clock is `performance.timeOrigin + performance.now()`, the
  machine's clock, in ms.
- A timeline's `at` is in its keeper's clock. Members convert it with their
  estimate of how far that clock is from their own (`offset`). A new keeper
  converts the timeline into its own clock when it takes over.
- Tabs of one browser share one clock, so their offset is 0.
- Over the network, every pair pings (4/s directly; through the service, 4/s
  until settled, then 1/s).
  - Each ping gives `offset = t1 − (t0 + t2)/2`.
  - The estimate is the mean of the quickest quarter of the last 16, eased in
    by 20%.
  - A timeline waits until its keeper's clock has 4 pings, or 3 s have passed.

**Into the engine.**
- A timeline is anchored on the audio frame that will be heard at its moment,
  from `getOutputTimestamp`. `SetTimeline` is frame-exact, like a note-on.
  A tempo change lands on the same frame.
- A timeline that arrives late is anchored where it was meant to be, through a
  signed frame delta.
- A clip started with the transport begins at the song position,
  `floor(beat / len) · len`.

**Staying on it.** Once a second, each member checks that the frame it will
hear 200 ms from now matches the timeline. It re-anchors when the two are
0.5 ms apart:
- **on two checks running**, for the audio clock's own drift (a few ms a
  minute) or a changed output latency. The browser's output time sometimes
  jumps for a moment and comes back, and a single check would chase that.
- **at once**, when the keeper's clock estimate has moved.

**Nudge.** Up to ±300 ms per tab, for output latency the browser doesn't
report (Bluetooth speakers).

## The network

**The link service** is at `/sync` on the site: a WebSocket (and
`/sync/health`). In production it runs beside nginx in the site's container,
started by `deploy/40-link-service.sh`, with nginx forwarding `/sync` to
`127.0.0.1:8002`. In development, the Vite dev and preview servers host it.

**Introductions.**
- Instances from the same address see each other: the IPv4 address, or the
  IPv6 /64. nginx passes it on as `X-Forwarded-For`, taking it from the
  proxy in front when that's on a private address (the Cloudflare tunnel, or
  the HTTPS proxy of a self-hosted copy). `CF-Connecting-IP` isn't read: any
  visitor could send it.
- Private and loopback addresses count as one local network.
- A group code puts together everyone who gives it, from any network.

**What the service does.**
- It passes WebRTC setup between peers.
- It relays messages for pairs that have no direct connection.
- It stores nothing.
- Limits: 16 per group, 100 messages/s per client (bursts of 200), 32 KB per
  message. Pages of other sites are refused (the Origin must match the Host).

**Direct connections.**
- The lower id offers.
- There are no STUN or TURN servers, so only local candidates are used. That
  is enough on one network, and nothing goes to an outside server.
- Two negotiated data channels: `msg` (reliable, ordered) for the group, and
  `ping` (unordered, no retransmits).
- Until `msg` opens, or if it never does, messages go through the service.
  The offering side tries again after 8 s, doubling to a minute.

**Two transports, one channel.**
- Every message goes out to the tabs and to the network.
- Each sender numbers its messages. A receiver takes each one once, and drops
  anything older than what it already has from that sender.
- What's dropped as late is sent again anyway: heartbeats carry the timeline,
  and requests are retried.

## Gates

Automated (`npm test`):

| Gate | Test |
| --- | --- |
| The engine starts, joins mid-song, anchors late timelines and holds synced LFO phase through tempo changes | `crates/engine/src/tests/p9.rs` |
| One keeper; anyone can play, stop and change tempo; late joiners; handover on leave or silence; groups merging | `tests/sync.test.ts` (sync group) |
| Clocks seconds apart, each estimated within ±0.2 ms: every member's beat at one moment agrees within 0.45 ms, through a handover and a tempo change | `tests/sync.test.ts` (across machines) |
| A newcomer whose clock is a minute behind doesn't take over; a missed change is caught up from the next heartbeat | `tests/sync.test.ts` |
| One copy of each message, in order, from either transport | `tests/sync.test.ts` (link channel) |
| Clock estimate: within 0.3 ms on a LAN (5 seeds); follows a 100 ppm drift within 0.4 ms; ignores queued pings; within 2 ms through the service | `tests/link-clock.test.ts` |
| Service: grouping by network (IPv4, IPv6 /64, private), group codes, passing messages only within a group, one connection per id, full groups, flood limit, Origin check, health | `tests/relay.test.ts` |
| Through the service end to end, with clocks 5 s apart: offsets within 2 ms, and one beat within 2 ms | `tests/net.test.ts` |

In the browser pane:
- `http://localhost:5173` and `http://[::1]:5173` are different origins, so
  they share no BroadcastChannel or storage and can only link over the network.
- With Link and Network on in both, each lists the other as `direct`, with a
  round trip under 1 ms and an offset under 0.3 ms.
- Play and tempo from either side reach both.
- Reloading the keeper hands over within 350 ms, and the beat at a fixed
  moment moves less than 0.05 ms.
- `__synth.link.corrections().log` shows re-anchors, and should be quiet after
  startup.

Two computers (after a deploy):
- With Link and Network on at https://birdsynth.abusing.technology on each,
  each lists the other as `direct`.
- Played together, the clips' notes land together by ear.
- Nudge whichever one is late on Bluetooth.

## Limits

- Two computers on one network may reach the site over different IP families,
  one over IPv4 and one over IPv6. The service then puts them in different
  groups: give both the same group code.
- Networks that block multicast DNS (some offices, guest Wi-Fi) hide Chrome's
  local candidates, so there's no direct connection. Messages then go through
  the service, which is slower and less even, so clock estimates are good to a
  millisecond or two.
- The browser's output latency is what it reports. Where that's wrong, use the
  nudge.
- When two groups that both kept playing meet, one of them jumps to the
  other's timeline: the one with more changes behind it.
