## Installation

### npm (recommended)

```bash
npm install -g @rover/agent-cli --registry https://jp1-artifactory.stargate.toyota/artifactory/api/npm/rover-npm-release/
```

### Direct download

Replace `<target>` with your platform (e.g., `aarch64-apple-darwin` or `x86_64-unknown-linux-gnu`).

```bash
# 1. Download the archive and its checksum
curl -sLO https://github.com/sg-genai/genai-cli/releases/download/TAG/agc-<target>.tar.gz
curl -sLO https://github.com/sg-genai/genai-cli/releases/download/TAG/agc-<target>.tar.gz.sha256

# 2. Verify the checksum
shasum -a 256 -c agc-<target>.tar.gz.sha256

# 3. Extract and install
tar -xzf agc-<target>.tar.gz
chmod +x agc
sudo mv agc /usr/local/bin/
```

---

