# LanPulse Test Record Template

Use one file per device pair, network, and build. Do not record PINs, device keys, session keys, or packet payloads.

## Build

- Date:
- Desktop commit:
- Android commit:
- Desktop OS and version:
- Android device and version:
- Router / AP:
- Test location:

## Audio Path

- Desktop audio source:
- Android output route:
- Sample rate / channels / packet ms:
- Volume settings:
- Background apps:

## Network Conditions

- Baseline Wi-Fi RSSI:
- Baseline ping:
- `tc netem` interface:
- `tc netem` preset or command:
- Other network load:

## Scenarios

| Scenario | Duration | Expected Result | Actual Result | Pass |
| --- | --- | --- | --- | --- |
| Discovery | 5 min | Desktop appears and stays current |  |  |
| QR pair | 5 min | Pairing succeeds with valid PIN |  |  |
| Foreground playback | 30 min | No disconnect, bounded buffer |  |  |
| Background playback | 30 min | No disconnect, bounded buffer |  |  |
| Lock-screen playback | 30 min | Media state remains correct |  |  |
| 1% loss | 10 min | Playback continues without reconnect loop |  |  |
| 500 ms pause | 5 runs | Recovers within 1 s after network returns |  |  |
| Explicit disconnect | 5 runs | Does not auto-reconnect or auto-play |  |  |

## Measurements

- Acoustic latency P50 / P95 / P99:
- Target buffer P50 / P95 / P99:
- Actual queue depth P50 / P95 / P99:
- Packets received:
- Packets lost:
- Late packets:
- Out-of-order packets:
- Duplicate packets:
- Silent packets inserted:
- Queue drops:
- AudioTrack underruns:
- Drift insertions:
- Drift drops:
- Reconnect count:
- Last disconnect reason:

## Device Health

- Android battery start / end:
- Android temperature start / peak / end:
- Desktop CPU:
- Android CPU:
- Memory notes:

## Notes

-
