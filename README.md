# Geostationary

A Bevy-based project.

## TODO to GitHub Issues Workflow

This repository includes an automated workflow that converts TODO items from a `TODO.md` file into GitHub issues.

### How It Works

When you push a `TODO.md` file to the `main` branch, the workflow automatically:

1. **Parses TODO items** - Reads markdown checkbox items (e.g., `- [ ] Task description`)
2. **Captures context** - Uses markdown headers (e.g., `# Feature Development`) as context
3. **Creates issues** - Generates a GitHub issue for each TODO item with the "todo" label
4. **Cleans up** - Deletes `TODO.md` and commits the change with `[skip ci]` to prevent workflow loops

### TODO.md Format

See `TODO.md.example` for a complete example. The basic format is:

```markdown
# Feature Category

- [ ] First task to do
- [ ] Second task to do
- [x] Completed task (lowercase x)
- [X] Also supports uppercase X

# Another Category

- [ ] Task under different context
```

### Features

- ✅ Supports both unchecked `- [ ]` and checked `- [x]` or `- [X]` items
- ✅ Includes context from markdown headers in issue descriptions
- ✅ Properly handles failures - won't delete TODO.md if issue creation fails
- ✅ Uses `[skip ci]` flag to prevent infinite workflow triggers
- ✅ Adds "todo" label to all created issues

### Usage

1. Create a `TODO.md` file in the repository root
2. Add your TODO items using markdown checkbox format
3. Commit and push to the `main` branch
4. The workflow will automatically create issues and delete the file

---

## Development

This is a Rust project using the Bevy game engine.

### Building

```bash
cargo build
```

### Running

```bash
cargo run
```
