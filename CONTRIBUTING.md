# Contributing to NDS

Thank you for your interest in contributing to NDS! We welcome contributions from the community.

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally
3. Create a new branch for your feature or bug fix
4. Make your changes
5. Run tests to ensure everything works
6. Commit your changes
7. Push to your fork
8. Create a Pull Request

## Development Setup

```bash
# Clone your fork
git clone https://github.com/yourusername/nds.git
cd nds

# Build the project
cargo build

# Run tests
cargo test

# Format code
cargo fmt

# Run linter
cargo clippy
```

## Code Style

- Follow Rust standard formatting (use `cargo fmt`)
- Write clear, self-documenting code
- Add comments for complex logic
- Keep functions small and focused
- Use meaningful variable and function names

## Testing

- Write tests for new functionality
- Ensure all tests pass before submitting PR
- Include both unit and integration tests where appropriate

## Commit Messages

We follow conventional commit format:

- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation changes
- `style:` Code style changes (formatting, etc)
- `refactor:` Code refactoring
- `test:` Adding or updating tests
- `chore:` Maintenance tasks

Example: `feat: add session export functionality`

## Pull Request Process

1. Update the README.md with details of changes if needed
2. Ensure your PR description clearly describes the problem and solution
3. Link any relevant issues
4. Request review from maintainers
5. Make requested changes if any
6. Once approved, your PR will be merged

## Reporting Issues

When reporting issues, please include:

- Operating system and version
- Rust version (`rustc --version`)
- Steps to reproduce the issue
- Expected behavior
- Actual behavior
- Any error messages or logs

## Feature Requests

We welcome feature requests! Please:

- Check if the feature has already been requested
- Provide a clear use case
- Explain why this feature would be useful
- Be open to discussion about implementation

## Code of Conduct

Please be respectful and considerate in all interactions:

- Be welcoming to newcomers
- Be respectful of different opinions
- Accept constructive criticism gracefully
- Focus on what's best for the project
- Show empathy towards other contributors

## Questions?

Feel free to open an issue for any questions about contributing.

Thank you for helping make NDS better!