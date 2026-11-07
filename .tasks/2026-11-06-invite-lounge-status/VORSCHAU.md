# Preview: invite lounge status messages

The Brain writes every text through its central provider and delivers it through the dl-bot broker. There are no fixed strings, so the wording below is the intent the Brain receives, not a template. Each message is a reply to the request post that mentions only the author (`allowed_mentions.users=[author]`, `parse: []`, `replied_user: false`). Without a channel the Brain sends a DM instead.

| State (`InviteLoungeUpdate`) | Trigger | Intent |
|---|---|---|
| `Status{FriendshipMissing}` | Steam bot sent a friend request | FA ist raus, Einladung folgt nach Annahme |
| `Status{Sent}` | Steam bot or Steam log reports the invite | Einladung ist raus |
| `Status{AlreadyHasGame}` | Steam reports an earlier invite, or one is on record at the 30 minute check | Schon eingeladen, keine neue Einladung nötig |
| `Problem{...}` | Invalid code, Steam rejected, friend request not accepted | Konkrete Ursache in klarer Sprache |
| `CheckIn` | 30 minutes, only in #deadlock-invite, nobody replied, no invite on record | Frage, ob die Einladung angekommen ist |
| `CheckInAnswer{outcome}` | Author answers the check-in; the Brain judged arrived, not_arrived or unclear; the answer text is passed along | Bestätigung, erneute Einladung oder Rückfrage |

Silent: Steam slow or unreachable, unclear Steam answers, requests posted before the first start of this version. The hint reply for requests without a code is unchanged.
