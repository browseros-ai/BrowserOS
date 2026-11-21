//! The one server-sent-events reader the three hand-written wires share: a streamed `reqwest`
//! body in, one `data:` payload at a time out (`eventsource-stream` underneath, or about 60
//! lines of its own). Chunk `models` fills this file.
