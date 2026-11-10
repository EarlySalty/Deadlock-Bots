# Support-panel handoff

This replaces the obsolete quota-stop note from November 9.

- Implementation: three Components V2 messages on branch `feat/support-panel-20261108`.
- Final German copy and buttons: [FINAL-PANEL.md](FINAL-PANEL.md).
- Lifecycle behavior and release prerequisites: [README.md](README.md).
- Astra reviewed candidate `bcefd3c7` and the runner candidate `17d73ae`. Its two blockers were the moderation-rights denial and unconditional missing-role advice. Both have been corrected in this working session.
- Targeted validation and a fresh Astra recheck are required before declaring the corrected commits reviewed.
- The local merge-gate reviewers failed to produce a valid verdict because of authentication/quota errors. Astra review does not itself update that gate.
- No deployment, live apply or Discord post is authorized.
