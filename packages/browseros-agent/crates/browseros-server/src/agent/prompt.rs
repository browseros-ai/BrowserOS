//! The system prompt, the wrapping of the user message in the model's copy (browser context,
//! selected text, `<USER_QUERY>`), and the text handed to a coding agent. Chunk `loop` fills
//! this file (design section 3.3, "The system prompt").
