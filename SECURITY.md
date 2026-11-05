# Security

Security fixes target the latest development version and latest release. There
is no commitment to backport fixes or to a response deadline.

Do not post vulnerabilities, secrets, or sensitive logs in public issues. Use
[GitHub private vulnerability reporting](https://github.com/SteliosDaVitzi/vijideck/security/advisories/new)
if enabled. Otherwise, contact the maintainer privately at
stelios_vitzi@hotmail.com to arrange disclosure before sending sensitive details.

Include the affected version or commit, OS, reproduction steps with disposable
data, impact, and any suggested fix. Relevant areas include cleanup boundaries,
process identity checks, command execution, log access, local storage, and Docker
context handling. Avoid testing against systems or data you do not own.

VijiDeck can display sensitive process arguments, environment values, logs, tool
paths, and Docker information. Its local SQLite cache is not a secrets vault.
Review copied details and screenshots before sharing them.
