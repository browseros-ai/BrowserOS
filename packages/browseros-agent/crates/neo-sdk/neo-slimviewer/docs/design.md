# How slimviewer works

## From X11 to the page

The X11 event thread wakes the engine when XDamage reports a change. The engine
reads the screen through shared memory and compares rows with the last picture.
If every row matches, it skips encoding. A resize recreates the capture buffer
and the encoder.

The engine owns the encoder and each viewer's frame acknowledgements. It uses
the owner's acknowledgements to pace capture, or a spectator's when there is
no owner. During motion it can reduce resolution or quality to fit the CPU and
link budget. After motion stops, OpenH264 sends a sharp full-size keyframe.

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
