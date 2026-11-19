# How slimviewer works

## From X11 to the page

The X11 event thread wakes the engine when XDamage reports a change. The engine
reads the screen through shared memory and compares rows with the last picture.
If every row matches, it skips encoding. A resize recreates the capture buffer
and the encoder.

The engine owns the encoder and each viewer's frame acknowledgements. It uses
the owner's acknowledgements to pace capture, or a spectator's when there is
no owner. The link and CPU each choose a frame-rate limit, and the lower one
paces capture. By default they step down through 30, 20 and 15 fps under pressure. At
the floor, a congested link raises the quantizer by up to two per step, to
+6 by default. Sustained congestion can then trigger half size. During congestion,
three or more large, full-size, non-key frames let the link estimate their mean
size at the quality limit. The link jumps to that limit if the mean frame takes
over 300 ms to deliver or if those frames at the interval's pace need more than
1.5 times that second's delivered bitrate. If delivery drops below half the
previous congested second's rate and that sample is at most three seconds old,
the bitrate comparison uses the previous rate. The same checks at the limit
select half size.
Two consecutive congested seconds without delivery can also accelerate the steps.
A late queue alone takes the usual quality steps. At the limit, two consecutive
congested seconds with a median ACK delay over 300 ms above the pacing RTT floor
can trigger half size.

Dense scrolling exposes more new rows per frame when capture slows. On a
pressure tick with enough full-size non-key motion frames (minimum below), the
link keeps its maximum fps if the mean frame takes over 150 ms at the dip-guarded
delivery rate. Bitrate overload then steps quality by up to two at a time.
If the link blocks pending damage longer than capture has credit and sends fewer
than half the expected frames during that active time, it also keeps maximum
fps. It estimates demand at the quality cap from the mean size of all full-size
frames sent, including keys, at the interval's pace. Above 1.5 times the
dip-guarded delivery rate, the link jumps to the cap, or to half size if
already at the cap. If the next tick after a jump is still starved and sends
zero frames, it selects half size. If delivery falls below half the preceding
tick's rate and its sample is at most three seconds old, frames that took at
most 150 ms each on average at that rate follow the usual link rules.
The usual jam tests select half size at the cap, still at maximum fps unless
the CPU limits it. Recovery skips the fps steps and clears this mode at base
quality and full size. Without a fresh probe, a motion return uses the last
congested delivery for 20 seconds after congestion.

Slow measured work can lower the CPU limit. Throttle or steal counts only when
capture falls below 80% of the current cadence during at least half a second
with damage pending and the pacer open. Grabs that find unchanged pixels count.
Recovery takes one step at a time. To raise quality or frame rate, the link needs
at least five large, non-key motion frames per clean second at 15 fps and above.
Below 15 fps, the minimum is one third of the paced frame rate, rounded down,
with a floor of one frame. Small updates between swipes preserve any clean
seconds already earned. A return to full-size motion must also fit the link's
300 ms keyframe budget. Burst probes subtract
the lowest RTT seen in the preceding five minutes. After congestion, the link
caps the latest probe at the measured congested delivery rate to price the
keyframe and the sustained bitrate of a full-size return. If half-size frames
are too small to probe, the last measurement still applies. Two consecutive
uncongested seconds with delivery and a median ACK delay less than 40 ms above
the base RTT release these limits.
After 20 seconds without congestion, one such second suffices. Fresh probes
continue to price full-size returns.
If the full key exceeds that budget, a full second without a change allows a
plain full-size frame through. Its acknowledgement measures fresh capacity.
A sharp settle frame must fit 300 ms. It uses the capped rate until the cap
lifts or ten seconds pass without congestion. It then uses the fastest sample
from the last ten seconds, or 8 Mb/s if none remain.

When an owner's acknowledgement window times out, the engine applies the
quality cap before encoding the next keyframe. Forced keys use half size whenever
the link or CPU has selected it, even for small changes. It measures delivered bytes,
CPU load and capture counts on the regular one-second tick.

The host relays a TCP byte stream over WebSocket. Each record starts with a
four-byte big-endian length covering a type byte and its payload. A record may
span multiple WebSocket messages. The page reassembles records, decodes Annex B
H.264 with WebCodecs, draws to a canvas and acknowledges the frame.

The FRAME header carries the sequence number (u32), flags (u8), and coded width
and height (u16 each). Bit 0 marks a keyframe and bit 1 a sharp frame. For a
half-size frame (bit 2), the page doubles the coded dimensions, then adds two
pixels to the width if bit 4 is set and to the height if bit 5 is set. This
recovers the full-size canvas used for field coordinates. Older pages ignore
bits 4 and 5. Older servers leave them clear. The page's probe reserves bit 3
for two-thirds size.

## From the page to Neo

The port selects the role. Spectators can acknowledge frames and request a
keyframe. The owner can also send pointer, keyboard and clipboard commands. An
input worker keeps those commands in order while capture continues. It sends
pointer events and keysyms over RFB. For text and paste it can also use Neo's
local DevTools endpoint to check focus and deliver text to the active field.

When ownership changes, the worker drains accepted input, releases held keys
and buttons, and waits for Xvnc's RFB fence before closing the old connection.
Clipboard contents and field updates go only to the current owner.
