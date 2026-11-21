//! A coding agent through the acpx command-line tool: one named session per conversation,
//! `prompt --file -` as a child process whose stdout ACP lines become `TurnEvent`s, and stop as
//! a second short-lived `cancel` child. ACP's JSON never leaves this file. Chunk `acp` fills
//! this file (design section 3.4).
