//! Provider failures into the extension's `ChatError`: classified from the HTTP status first and
//! the message second, retry hints read, upstream text scrubbed of keys and cut at 20,000
//! characters before it becomes `details`. Chunk `models` fills this file.
