# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability within NDS, please:

1. **Do not** open a public issue
2. Send a description of the vulnerability to the maintainers through GitHub's private vulnerability reporting feature
3. Include steps to reproduce if possible

We take all security bugs seriously and will respond as quickly as possible to fix the issue.

## Security Considerations

NDS handles shell sessions and PTY allocation. When using NDS:

- Sessions run with the permissions of the user who created them
- Session metadata is stored in `~/.nds/` with user-only permissions (0700)
- No network connectivity or remote access features are implemented
- All IPC happens through Unix domain sockets with appropriate permissions

## Best Practices

- Keep NDS updated to the latest version
- Regularly clean up old sessions with `nds clean`
- Be cautious when attaching to sessions you didn't create
- Review session history periodically with `nds history`