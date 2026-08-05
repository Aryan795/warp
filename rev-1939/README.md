# REV-1939 — reviewer-visible visual proof

Disposable asset branch. It carries **no source code** and is not part of any
pull request diff. It exists only so the computer-use proof for
[#14713](https://github.com/warpdotdev/warp/pull/14713) renders inline for a
reviewer who is not signed in to Oz. Delete the branch once #14713 is merged or
closed.

| File | Arm | What it shows |
| --- | --- | --- |
| `choose-how-to-start-control.png` | Control | Two cards — "Use Warp with AI" (Recommended) and "Set up AI later" — with purchasable credit packs loaded, and no credit tiles. |
| `choose-how-to-start-experiment.png` | Experiment | Three cards — "Subscribe to a Warp plan" (Recommended), "Buy AI credits" with the 400/1,000/3,000/6,500 tiles, and "Set up AI later". |
| `choose-how-to-start-unassigned.png` | Unassigned | The same two-card fallback the control arm renders. |
| `choose-how-to-start-all-arms.gif` | All three | The same 11s walkthrough as an animated GIF, which is what renders inline on the pull request. |
| `choose-how-to-start-all-arms.mp4` | All three | 11s recording of the three runs, with arrow-key navigation in control and experiment and a credit-tile click in experiment. |
| `choose-how-to-start-all-arms-720p.mp4` | All three | A 109 KB 720p transcode of the same recording, for a quick download. |

The captures are the unmodified Oz computer-use artifacts, re-uploaded here
byte-for-byte. The control and unassigned stills are pixel-identical, which is
the behaviour the spec requires: an unassigned user gets exactly the historical
control layout.
