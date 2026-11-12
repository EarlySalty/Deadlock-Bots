# Gate retro: invite lounge chat judge (2026-11-12)

Scope: Brain branch `feat/invite-handled-judge-20261111` (worktree `/home/nathanael/.worktrees/brain-invite-handled-20261111`, pushed as `feat/invite-handled-judge-r3-20261111`, head 850e0d27) plus the Bots branch `feat/invite-lounge-chat-judge-20261111` (head 68da79c0) that pins that Brain rev. The gate counter for the Brain branch stands at 4. Renamed branches count as the same branch; the next round is the final one.

## 1. Why four rounds

| Round | Head | Findings | Kind |
|---|---|---|---|
| 1 | 9da27d47 | 5 (free-text `ChatLine.author` labels the prompt gives meaning to; error paths) | new |
| 2 | ba9f8968 | multi-line message text fakes a new speaker line (LF) | new, same risk |
| 3 | 30f449c6 | lone CR, U+2028, other format! sites still raw; char cap vs byte cap | incomplete fix |
| 4 | 936f35c0 | 31: labels (author, login, title) still pass U+2028/U+2029; `fits_request_limit` duplicates the client check and has no caller | incomplete fix + regression by fix |

All four rounds circled one risk: the Brain renders caller text as `Role: text` lines, so text that contains a line terminator (or a label that does) can start a line that looks like a bot-set speaker line. Each fix closed the characters and sites the last review named (LF, then CR/U+2028/NEL, then labels) and the next review found the next member of the same set. Cause of the round count: the defence was a deny-list of line terminators applied site by site, not a rendering in which caller text cannot form structure. The byte-cap helper was added without searching for the existing client check (regression caused by the fix). No scope creep and no reviewer drift.

## 2. Triage of open findings (round 4, on 936f35c0; 850e0d27 not yet reviewed)

| Class | Findings | Verdict | Reason |
|---|---|---|---|
| A1 speaker-line injection through line terminators in labels and texts | #60-#67, #69-#76, #82-#84, #88 | REAL | Reachable from member input; 850e0d27 widens the deny-list again, which is the same incomplete approach. |
| A2 `fits_request_limit` duplicates the client byte check, no production caller | #68, #79, #85, #86, #90 | REAL | Two copies of one limit rule; 850e0d27 routes the client through it and the bot trims with it, needs re-review only. |
| A3 test named "trimming fits it" never reaches production byte checks | #81, #87 | REAL | Test cannot fail on client or server drift; must exercise the client path. |
| A4 Ursache of 936f35c0 misstates "no producer-side byte check existed" | #89 | REAL | Misleading commit text; corrected in the new commit's Ursache block (no history rewrite, no force push). |
| A5 hook stale_state markers on persona/test units | #70-#78 | TRIVIA (follow A1) | Hook marks derived from the A1 finding; they close with A1, no separate defect. |

## 3. Fix plan

A1, structural: every caller-supplied value that persona.rs places in a prompt next to bot-set text is rendered as JSON (serde_json), never as raw text after a role prefix. Chat lines become `{"autor": ..., "text": ...}` objects, dialog turns `{"rolle": ..., "text": ...}`, the invite judge conversation `{"rolle": <closed enum label>, "text": ...}`, the stream block an object, single messages JSON strings. JSON escapes every quote and control character, so text can never close its string and name a speaker; no character list is needed. The deny-list machinery goes away: `continued`, `messages`, `quoted_judge`, the delimiter stripping, the exported `line_break` and the `single_line` label validator (labels go back to the main rule, they no longer need to be single-line for prompt safety). Prompt texts that describe the old line or `<<<...>>>` block format (`invite_handled_judge.txt`, `invite_check_in_judge.txt`, `lfg_judge.txt`) are rewritten to describe the JSON. One test feeds every terminator and a forged speaker object through each renderer and parses the data back to prove each value stays one JSON value. Prompts whose whole user turn is a single caller value with no bot-set line after it (`pate`, `voice_hint`, `tip`) have no speaker structure to forge and stay as they are.

A2/A3: keep 850e0d27's single rule (`BotTaskRequest::fits_request_limit`, client calls it); the contract test drives the client size check instead of only the helper.

A4: the new commit's Ursache block states the real gap (no trim strategy, the client check already existed).

Bots side: bump the dl-brain rev to the fixed Brain head; the bot already sends closed speaker roles and trims with `fits_request_limit`.

## Second opinion (Fable, 2026-11-12) and adjustments

- Labels still reach system prompts raw (`persona.rs` chat_system streamer_login, dashboard name and page, analysis name). So label validation stays single-line (control characters plus U+2028/U+2029, the only line terminators outside Cc); the exported `line_break` list is gone.
- JSON values between German headings are not enough: the whole user turn must be one JSON document, with bot-set facts as keys (`neu_im_chat`, `deadlock_live`, `angehaengte_bilder_fuer_den_antwortschritt`), and every control or non-space whitespace character escaped so the rendered turn has no line break at all. Done.
- The check-in judge goes back byte for byte to main (one raw value, no speaker structure, same shape as pate/voice_hint/tip). Done.
- Size rule: one method that returns the body (`BotTaskRequest::body_within_limit`); serialization errors propagate instead of turning into RequestTooLarge. Done; the client and a client-path test use it.
- Stale Ursache blocks: 936f35c0 and 850e0d27 claim things HEAD no longer does. Plan: squash the Brain branch locally into one commit with an accurate Ursache block (same local branch name, counter stays at 4, no force push; publish only as HEAD:main after ALLOW).
- Bot trim (`dl-brain/src/brain_api.rs` fit_request_limit) drops the oldest lines, which are the replies right after the request. Plan: drop from the middle so the first replies and the latest state both stay; same rule for the line cap.
- Audit outcome changed from `judged` to `judged_*`: no reader in Brain or Bots; check other repos before merge.
- The live prompts of Twitch chat, passive help, concierge, promo and the judges change shape. Needs a runtime check on real chats after deploy.
- Triage fixes: #77/#78 belong to A2; #80 joins A1; A5 lists #70-#76 only.
