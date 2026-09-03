# Screenshot Search — Security & Privacy Context

## 1. Security Model

Screenshot Search processes highly sensitive local data.

Screenshots may contain:

- passwords
- API keys
- tokens
- banking data
- private messages
- source code
- email addresses
- private documents
- personal photographs
- authentication QR codes
- internal company information

Privacy and security are core product requirements.

---

## 2. Default Data Boundary

Default architecture:

```text
Screenshot
    |
    v
Local OCR
    |
    v
Local Database
    |
    v
Local Search
    |
    v
Local AI
```

Network transfer:

```text
none required
```

The app must not upload screenshots or OCR text by default.

---

## 3. Original Files

Rules:

- never overwrite the user's screenshot
- never move screenshots automatically
- never delete screenshots automatically
- opening/revealing originals must be explicit user actions
- indexing cleanup must affect only application-owned data

---

## 4. OCR Data

OCR text is sensitive.

Rules:

- store locally
- do not include full OCR text in logs
- do not include OCR text in crash reports
- do not transmit to analytics
- do not include OCR text in telemetry event properties

---

## 5. Filesystem Paths

Paths can reveal user identity or organization structure.

Example:

```text
C:\Users\<username>\Company\SecretProject\
```

Logs should avoid unnecessarily storing complete private paths.

When diagnostics require a path:

- consider redaction
- consider hashing
- expose it only locally
- obtain explicit consent before sharing diagnostics externally

---

## 6. Logs

Never log:

- OCR full text
- screenshot bytes
- access tokens
- passwords
- API keys
- database contents
- embeddings if not necessary
- full sensitive request payloads

Use structured error codes.

Preferred:

```text
OCR_FAILED
image_id=123
engine=paddle
```

Avoid:

```text
OCR failed for screenshot containing:
"my password is ..."
```

---

## 7. Database

SQLite database contains sensitive derived content.

Rules:

- keep in application data directory
- use safe permissions available from the OS
- no automatic cloud upload
- no hidden backup to developer infrastructure
- database export must be explicit

Future optional encryption-at-rest should be investigated if threat model requires it.

Do not claim database encryption exists unless actually implemented.

---

## 8. Telemetry

If telemetry is added:

Safe examples:

```text
app_version
OS version
indexing duration
number of indexed screenshots
generic error code
feature usage count
```

Potentially unsafe:

```text
OCR text
search query
full file path
screenshot filename if sensitive
screenshot bytes
semantic query contents
```

Search queries can be highly sensitive.

Do not collect them by default.

---

## 9. Crash Reporting

Crash reporting must be privacy reviewed.

Before enabling:

- inspect payload
- redact filesystem paths
- ensure OCR text cannot enter breadcrumbs
- ensure search queries are excluded
- ensure screenshots are never attached automatically

---

## 10. AI

Local AI is preferred.

If a cloud AI feature is added:

The UI must explicitly communicate:

- what data is sent
- which provider receives it
- why it is needed
- whether screenshots or OCR text are included

Cloud processing must be opt-in.

Do not silently fall back to cloud AI when local inference fails.

---

## 11. Model Downloads

Local AI models may be downloaded from the internet.

Security requirements:

- HTTPS
- trusted source
- versioned model metadata
- checksum verification where practical
- do not execute arbitrary downloaded code
- models should be treated as data assets

---

## 12. File Parsing

Image decoders process untrusted local files.

Rules:

- use maintained libraries
- validate supported file types
- enforce reasonable size limits where needed
- handle malformed images safely
- never assume extension guarantees valid content

---

## 13. Tauri Permissions

Follow least privilege.

Only expose Tauri capabilities needed by the application.

Do not grant arbitrary filesystem access to frontend code.

Prefer scoped operations through trusted Rust commands.

---

## 14. External URLs

If screenshots contain URLs, do not automatically open them.

Require explicit user action.

Use the OS/browser safe external-open mechanism.

---

## 15. Clipboard

If adding Copy OCR Text:

- copy only after explicit user action
- do not continuously inspect clipboard unless a future feature explicitly requires it
- avoid leaving secrets in clipboard longer than necessary if implementing secure-copy features

---

## 16. Threat Model Questions

When adding a feature, consider:

1. Can this expose screenshot contents?
2. Does it increase filesystem permissions?
3. Does data leave the machine?
4. Can malformed files crash the app?
5. Can logs leak secrets?
6. Can a webpage displayed in the UI invoke native capabilities?
7. Can a malicious file/path escape expected directories?
8. Can a cloud dependency see user data?

---

## 17. Privacy Promise

Do not make marketing claims stronger than the implementation.

Only say:

> Screenshots never leave your device.

if every enabled default feature satisfies that statement.

If optional cloud features exist, communicate the distinction clearly.
