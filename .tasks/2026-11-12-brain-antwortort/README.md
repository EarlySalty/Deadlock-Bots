# Brain live reads: limit to what the answer place may see

## Problem

The Brain answers in public Discord channels with live Discord facts read through `/mcp/public`. dl-bot sends the asker's user id, so `public::access` computes the asker's own permissions. The facts then contain every channel the asker can see, private team, staff and mod channels included (name, topic), and `<#channel>` mentions read messages from those channels. The answer is posted publicly. Anyone in a private channel could make the Brain repeat its content in a public channel.

Mechanism: `facts_for` and `request_tool` (`read_messages`, `search_messages`) filter by `effective_permissions(channel, access)` of the requester only. The answer place is not known to the MCP.

## Fix

- `answer_places.rs`: dl-bot registers the answer place (`Requester` for DMs, `Channel(id)` for guild answers) per user for the lifetime of a Brain request (RAII guard). Used at the three Brain entry points in `modglue.rs` (public message, direct/private flow, slash command).
- `mcp/public.rs`: `answer_access` adds a ceiling to the requester's access. For every active `Channel(c)` place a channel must also be visible to the verified-member baseline, except `c` itself. `Requester` places add no ceiling. No registered place means baseline only (fail closed). `effective_permissions` applies the ceiling, so channel listing, `read_messages`, `search_messages` and channel-term reads all follow it.
- `send_message` and `self_invite_status` keep the plain requester access.

## Acceptance

- In a public channel, a member of a private team channel gets no private channel names, topics or messages in the Brain facts.
- In a DM, the requester still gets their own readable channels.
- Unit test `antwortort_begrenzt_private_kanaele_des_fragenden` covers the place combinations; `orte_gelten_nur_solange_die_anfrage_laeuft` covers the registry.

## Live check after deploy

`journalctl -u deadlock-bot-rust` shows the new PID. Then ask the Brain in a public channel about a private team channel (`@Brain was steht in <#team-channel>?`): the answer must not contain team channel content. The same question by DM to a team member may use it.
