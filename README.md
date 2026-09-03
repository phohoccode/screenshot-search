# Screenshot Search — AI Context Pack

This folder contains the context documentation intended to help AI coding agents such as Codex understand the Screenshot Search project quickly and safely.

## Read Order

For a new session, read:

1. `PROJECT_CONTEXT.md`
2. `CURRENT_STATUS.md`
3. `docs/CODING_RULES.md`

For any frontend/UI task, also read:

4. `docs/UI_DESIGN_SYSTEM.md`

Then read only the other context relevant to the task.

### Frontend / UI Task

Read:

- `docs/UI_DESIGN_SYSTEM.md`
- `docs/CODING_RULES.md`
- `ARCHITECTURE.md`

### Architecture Task

Read:

- `ARCHITECTURE.md`
- `docs/DECISIONS.md`

### Database Task

Read:

- `docs/DATABASE.md`
- `ARCHITECTURE.md`

### Search Task

Read:

- `docs/SEARCH_CONTEXT.md`
- `docs/DATABASE.md`

### AI / Semantic Search Task

Read:

- `docs/AI_CONTEXT.md`
- `docs/SEARCH_CONTEXT.md`
- `docs/SECURITY_PRIVACY.md`

### Security / Privacy Task

Read:

- `docs/SECURITY_PRIVACY.md`
- `docs/CODING_RULES.md`

---

## Recommended Prompt for a New Coding Session

```text
Before making any changes:

1. Read PROJECT_CONTEXT.md.
2. Read CURRENT_STATUS.md.
3. Read docs/CODING_RULES.md.
4. For any frontend/UI task, read docs/UI_DESIGN_SYSTEM.md before editing.
5. Read the other domain-specific context files relevant to this task.
6. Inspect the actual repository structure and source code.
7. Run git status and inspect relevant git diff.
8. Treat actual code, migrations, and tests as implementation source of truth.
9. Do not blindly trust context files if they are stale; update them when a meaningful architectural or project-status change is confirmed.
10. Respect the project's local-first and privacy-first constraints.
11. For UI work, preserve the Minimal SaaS / shadcn-style design system and reuse shared primitives.
12. Do not add cloud AI, external databases, or unnecessary infrastructure unless the task explicitly requires it and the architectural impact is justified.
```

---

## Files

```text
PROJECT_CONTEXT.md
CURRENT_STATUS.md
ARCHITECTURE.md

docs/
├── DATABASE.md
├── SEARCH_CONTEXT.md
├── AI_CONTEXT.md
├── SECURITY_PRIVACY.md
├── CODING_RULES.md
└── DECISIONS.md
```

---

## Maintenance Rule

Do not turn context files into source-code dumps.

Keep them focused on:

- project intent
- current state
- architecture
- invariants
- decisions
- safety/privacy constraints

The repository source code remains the implementation source of truth.
