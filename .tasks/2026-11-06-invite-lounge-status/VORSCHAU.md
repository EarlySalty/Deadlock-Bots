# Preview: invite lounge status messages

Every status message is a Discord reply to the request post. It mentions only the author (`allowed_mentions.users = [NUTZER_ID]`, `parse: []`, `replied_user: false`). `<@NUTZER_ID>` stands for the author's mention. The status texts are fixed Brain templates (`brain-api/src/persona.rs`, task `discord_invite_status`) with no model call. The 30-minute question and the reply to the author's answer are written by the Brain (`invite_check_in_judge`), which also judges the answer as arrived, not_arrived or unclear. The German examples below show the intended wording, not fixed strings. Each state is posted at most once per request.

Example request (room): `Kann mich jemand einladen? <FREUNDESCODE>`
Example request (direct to the bot): `Kannst du mich einladen? <FREUNDESCODE>`

| State | Trigger | Text |
|---|---|---|
| FA ist raus | Steam bot answers 📨 (friend request sent) | <@NUTZER_ID> FA ist raus. Nimm die Freundschaftsanfrage in Steam an, danach kommt die Einladung automatisch. |
| Einladung ist raus | Steam bot answers 🎮, or after the FA the Steam log shows `playtest_invite` `sent` | <@NUTZER_ID> Einladung ist raus. Du findest sie in deiner Steam-Bibliothek oder unter https://store.steampowered.com/account/playtestinvites. |
| Bereits eingeladen | Steam bot answers ℹ️ or ✅, Steam log `sent` with code 5, or audit entry found at the 30 minute check | <@NUTZER_ID> Du bist schon eingeladen, eine neue Einladung brauchst du nicht. Schau in deiner Steam-Bibliothek oder unter https://store.steampowered.com/account/playtestinvites nach. |
| Problem: ungültiger Code | Steam bot answers ❌ | <@NUTZER_ID> Mit diesem Freundescode klappt die Einladung nicht. Der Steam-Freundescode besteht nur aus Ziffern, du findest ihn in Steam unter Freunde → „Freund hinzufügen“. |
| Problem: Steam lehnt ab | Steam bot answers "Steam hat die Einladung abgelehnt", or Steam log `rejected` | <@NUTZER_ID> Steam hat die Einladung für diesen Account abgelehnt, deshalb ist keine Einladung rausgegangen. |
| Problem: FA nicht angenommen | Steam log `timeout` after the friend request | <@NUTZER_ID> Die Freundschaftsanfrage wurde innerhalb von 24 Stunden nicht angenommen, deshalb ist keine Einladung rausgegangen. |
| 30-Minuten-Nachfrage | Room request with code, 30 minutes old, no reply to the post and no mention of the author by anyone else (human or bot), no invite on record | Brain writes it, mentioning the author. Intended wording: <@NUTZER_ID> Ist deine Einladung schon angekommen? Antworte kurz mit ja oder nein. Wenn nicht, schicke ich sie dir. |
| Antwort: angekommen | Author answers the check-in and the Brain judges `arrived` (confidence at least 0.7) | Brain writes the reply. Intended wording: <@NUTZER_ID> Alles klar, dann ist deine Einladung angekommen. Viel Spaß in Deadlock! |
| Antwort: nicht angekommen | Author answers the check-in and the Brain judges `not_arrived` | Brain writes the reply, the invite is sent again, and the result is reported with the states above. |

## Silent states

- Steam slow (⏳) or unreachable (⚠️ or transport error): no message. The request stays open; the watcher checks the Steam tables every minute after a friend request and retries the invite every 5 minutes when Steam was unreachable, within the 7 day window.
- Author's answer judged unclear (or Brain not reachable for the answer): no state change, the request stays open, and the Brain's reply to the author may ask whether the invite arrived in Steam friends or in Deadlock. A failed Brain call leaves the author's message unused, so a later answer is still judged.
- Unclear Steam bot answer: no message; the result is only stored.
- Requests posted before the first start of this version: no message and no invite.

The hint reply for requests without a code is unchanged (`parse: []`, no mention).
