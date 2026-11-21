//! The OpenAI Chat Completions wire, written over `reqwest`: `openai-compatible`, `ollama`,
//! `lmstudio`, `moonshot`, `fireworks`, `openrouter`, `azure` (its v1 route, `api-key` header) and
//! `google` (Gemini's OpenAI-compatible endpoint). Each assistant message's tool calls are kept
//! verbatim for the next step of the turn, including OpenRouter's `reasoning_details` and
//! Gemini's `extra_content` thought signature. Chunk `models` fills this file (CORRECTIONS-2).
