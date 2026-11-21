//! The stored transcript projected into the model's messages (earlier turns as text, tool parts
//! as one line each, reasoning dropped), compaction on the model's copy only, and pictures from
//! a tool result as a user message. Pure functions. Chunk `loop` fills this file.
