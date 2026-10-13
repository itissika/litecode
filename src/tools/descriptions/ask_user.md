Ask the user one or more multiple-choice questions and wait for answers.
Use when you need decisions, preferences, or confirmations only the user can give.
Pass questions: [{ id, prompt, options[{id,label}], multi_select? }] (at least one). Optional summary is a card title.
Per question: multi_select allows several picks. The system automatically appends a final entry 「其他（自己填写）」 to every question so the user can write their own answer; do not add your own "Other" / "其他" option.
On submit you receive answers keyed by question id: selected_ids and/or free_text. A free_text with no selected option means the user answered via 「其他（自己填写）」. On skip you get a clear no-answer result - do not invent choices.
Legacy single question+options still works (treated as question id q0). Prefer questions[].
Prefer this over guessing or blocking on chat. Not for tool permission grants (those are automatic Asks) or plan create (use plan).
