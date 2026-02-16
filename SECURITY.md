# Security Policy

This document defines how to report vulnerabilities in Openportio and what support window to expect.

## Reporting A Vulnerability

Preferred channel:
- Use GitHub private vulnerability reporting for this repository:
  - <https://github.com/jaeyoung0509/Openportio/security/advisories/new>

Please include:
- affected crate(s) and version/commit
- impact summary (confidentiality/integrity/availability)
- reproduction steps or proof-of-concept
- suggested fix direction (if available)

Do not open public issues for unpatched vulnerabilities.

If private reporting is unavailable for any reason, open a minimal public issue without exploit details and request a private contact channel from maintainers.

## Response Expectations

- Initial acknowledgement: within 72 hours
- Triage decision: within 7 calendar days
- Status updates: at least weekly for active incidents

Resolution timeline depends on severity and release risk, but critical issues are prioritized immediately.

## Supported Versions

Openportio is pre-1.0 (`0.y.z`). Security fixes are focused on the latest release line.

| Version / Branch | Security Support |
| --- | --- |
| Latest `0.y.z` release on `main` | Supported |
| `develop` branch | Best effort (may change quickly) |
| Older release lines | Not supported |

When feasible, fixes land on `develop` first and are included in the next release.

## Disclosure Process

- We follow coordinated disclosure by default.
- Report details are kept private until a fix or mitigation is available.
- Advisory publication timing is coordinated with patch release.
- Reporters are credited in advisories/releases when requested.

## Operator Notes

For runtime hardening guidance, see:
- `docs/production/security.md`
- `docs/production/deployment.md`
- `docs/production/runbook.md`
