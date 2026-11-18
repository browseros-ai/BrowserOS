# How slimviewer works

## From X11 to the page

The X11 event thread wakes the engine when XDamage reports a change. The engine
reads the screen through shared memory and compares rows with the last picture.
If every row matches, it skips encoding. A resize recreates the capture buffer
and the encoder.

The engine owns the encoder and each viewer's frame acknowledgements. It uses
the owner's acknowledgements to pace capture, or a spectator's when there is
no owner. The link and CPU each choose a frame-rate limit, and the lower one
paces capture. Under pressure they step down through 30, 20 and 15 fps. At
the floor, a congested link raises the quantizer by up to two per step, to
+6 by default. Sustained congestion can then trigger half size. A frame that
would take over 300 ms at the quality limit, late acknowledgements or stalled
delivery can accelerate those steps. Slow measured work can lower the CPU
limit. Throttle or steal counts only when frames miss 80% of the current
cadence during at least half a second with damage pending and the pacer open.
Recovery takes one step at a time after sustained headroom. A return to full-size
motion must also fit the link's 300 ms keyframe budget. Fresh capacity samples
set the budget. While half-size traffic is too small to measure, the last sample
continues to apply until the link has gone 20 seconds without congestion.
Once damage stops, the full-size settle frame can still go
out and its acknowledgement measures fresh capacity.

The host relays a TCP byte stream over WebSocket. Each record starts with a
four-byte big-endian length covering a type byte and its payload. A record may
span multiple WebSocket messages. The page reassembles records, decodes Annex B
H.264 with WebCodecs, draws to a canvas and acknowledges the frame.

## From the page to Neo

The port selects the role. Spectators can acknowledge frames and request a
keyframe. The owner can also send pointer, keyboard and clipboard commands. An
input worker keeps those commands in order while capture continues. It sends
pointer events and keysyms over RFB. For text and paste it can also use Neo's
local DevTools endpoint to check focus and deliver text to the active field.

When ownership changes, the worker drains accepted input, releases held keys
and buttons, and waits for Xvnc's RFB fence before closing the old connection.
Clipboard contents and field updates go only to the current owner.
