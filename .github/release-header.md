## Installation

### npm (recommended)

```bash
npm install -g a2a-protocol-cli
```

### Cargo

```bash
cargo install a2a-protocol-cli
```

### Direct download

Replace `<target>` with your platform (e.g., `aarch64-apple-darwin` or `x86_64-unknown-linux-gnu`).

```bash
# 1. Download the archive and its checksum
curl -sLO https://github.com/sandangel/a2a-cli/releases/download/TAG/a2a-<target>.tar.gz
curl -sLO https://github.com/sandangel/a2a-cli/releases/download/TAG/a2a-<target>.tar.gz.sha256

# 2. Verify the checksum
shasum -a 256 -c a2a-<target>.tar.gz.sha256

# 3. Extract and install
tar -xzf a2a-<target>.tar.gz
chmod +x a2a
sudo mv a2a /usr/local/bin/
```

---
