Ask the user one or more multiple-choice questions and wait for answers.
Use when you need decisions, preferences, or confirmations only the user can give.
Pass questions: [{ id, prompt, options[{id,label}], multi_select?, allow_free_text? }] (at least one). Optional summary is a card title.
Per question: multi_select allows several picks; allow_free_text (default true) adds an optional free-text field.
On submit you receive answers keyed by question id (selected option ids + optional free text). On skip you get a clear no-answer result - do not invent choices.
Legacy single question+options still works (treated as question id q0). Prefer questions[].
Prefer this over guessing or blocking on chat. Not for tool permission grants (those are automatic Asks) or plan create (use plan).
