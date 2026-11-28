# Deadlock Brain follow-up addressee

Keep public Discord follow-ups available when they are clearly addressed to Brain.
Preserve explicit mentions and replies. Game facts, guide routing, and the Brain answer service are outside this change.

Replay the `#🌐allgemein` exchange. Brain should answer the wave question, stay silent while the two members discuss a match, and answer a later wave follow-up without a new mention.

- Mechanism. The Discord history shows Brain replying to “Dann kann ich dir das Zeigen” and further match discussion after a question about souls.
- Enabling condition. The earlier event handler treated the same author and recent Brain reply as evidence of intent. The current fix adds question syntax and a conversation closure flag, which cannot determine the addressee of a question.
- Detection gap. The current regression test accepts a reply to Brain as the condition for reopening the conversation. It does not require a later untagged follow-up to work.
- Related paths. The dispatch prefilter and answer task both call the same conversation method. Keep candidate selection free of side effects in both. The guide and explicit command have their own eligibility rules.

Use a bounded excerpt of public channel history for untagged follow-ups. Keep usage accounting for the classifier and exclude its input and output from the Discord transparency feed.
