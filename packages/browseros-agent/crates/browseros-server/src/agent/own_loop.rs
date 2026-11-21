//! The server's own loop for an LLM provider: at most 100 steps and one hour; each step is one
//! `Model::step`, then every tool call that response asked for, one after another through
//! `Browser::call`. Stop is checked at the model call, the browser call and between steps.
//! Chunk `loop` fills this file (design section 3.3).
