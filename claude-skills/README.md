# claude-skills

Versionierte Heimat unserer eigenen **Claude-Code-Skills** (kein Bot-Code).
Hier landen selbstgebaute Skills, damit sie gesichert und nachvollziehbar sind —
die *aktiven* Kopien, die Claude Code lädt, liegen unter `~/.claude/skills/`.

## Installieren / Syncen

Eine Skill-Version aus diesem Ordner aktiv schalten — entweder kopieren:

```bash
cp -r claude-skills/<skill-name> ~/.claude/skills/
```

oder (empfohlen, bleibt automatisch in Sync) verlinken:

```bash
ln -sfn "$PWD/claude-skills/<skill-name>" ~/.claude/skills/<skill-name>
```

## Vorhandene Skills

| Skill | Zweck |
|-------|-------|
| `documenting-code-for-support-agents` | Erzeugt aus einer Codebasis eine hierarchische, redigierte HTML-Wissensbasis für KI-Support-Agenten. Dokumentiert Verhalten & Effekt, hält geheime Mechanik (Scoring-Schwellen, Preis-/Margen-, Ranking-, Fraud-, Admin-Logik, Secrets) konsequent draußen. RED/GREEN-getestet: 6/6 Leak-Kategorien geschlossen. |

## Konvention

Jeder Skill ist ein Unterordner mit `SKILL.md` (Frontmatter `name` + `description`)
plus optionalen Referenz-/Werkzeugdateien. Änderungen hier committen **und** die
aktive `~/.claude/skills/`-Kopie nachziehen (oder Symlink nutzen).
