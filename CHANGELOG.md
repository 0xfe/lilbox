# Changelog

## Unreleased

- Fix native macOS root certificate discovery in online profiles, which could
  cause Codex HTTPS/WebSocket connections to fail with `UnknownIssuer`.
- Preserve inherited proxy and CA-bundle settings for permissive networking.
- Add offline native-certificate regression coverage while retaining direct
  private-keychain file protection and network-deny enforcement.

## 0.1.0 — 2026-09-19

- Initial macOS Seatbelt backend and portable policy/backend interface.
- Direct agent, shell, and command launch with native exit/signal behavior.
- Open and closed profiles, credential path protection, environment scrubbing,
  private temporary storage, and network allow/deny modes.
- Strict versioned TOML configuration and non-overwriting profile initialization.
- Portable tests, macOS enforcement tests, Just recipes, CI, and release archives.
