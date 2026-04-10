BINARY  := agc
CMD     := ./cmd/agc
BASE_VERSION := $(shell node -p "require('./npm/package.json').version" 2>/dev/null || echo "0.0.0")
COMMIT       := $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")

CLI_PKG := genai.stargate.toyota/agc/internal/cli
ENV_PKG := genai.stargate.toyota/agc/internal/config

.PHONY: all build build-dev build-stg install test vet fmt clean build-all npm-pack npm-publish generate-skills

all: build-dev

# ── Local build ──────────────────────────────────────────────────────────────

# dev: version auto-computed as BASE_VERSION-dev+COMMIT, targets dev environment.
build-dev:
	go build \
	  -ldflags "-X $(CLI_PKG).version=$(BASE_VERSION)-dev+$(COMMIT) -X $(ENV_PKG).BuildEnv=dev" \
	  -o bin/$(BINARY) $(CMD)

# stg: VERSION must be provided and must be an RC tag (e.g. 1.2.0-rc.1).
build-stg:
	@[ -n "$(VERSION)" ] || (echo "Error: VERSION required  (e.g. make build-stg VERSION=1.2.0-rc.1)" && exit 1)
	@echo "$(VERSION)" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+-rc' \
	  || (echo "Error: stg VERSION must be an RC tag (e.g. 1.2.0-rc.1, got '$(VERSION)')" && exit 1)
	go build \
	  -ldflags "-X $(CLI_PKG).version=$(VERSION) -X $(ENV_PKG).BuildEnv=stg" \
	  -o bin/$(BINARY) $(CMD)

# prod / release: VERSION must be provided and must be clean semver (e.g. 1.2.0).
build:
	@[ -n "$(VERSION)" ] || (echo "Error: VERSION required  (e.g. make build VERSION=1.2.0)" && exit 1)
	@echo "$(VERSION)" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$$' \
	  || (echo "Error: prod VERSION must be clean semver with no pre-release suffix (got '$(VERSION)')" && exit 1)
	go build \
	  -ldflags "-X $(CLI_PKG).version=$(VERSION) -X $(ENV_PKG).BuildEnv=prod" \
	  -o bin/$(BINARY) $(CMD)

install:
	go install \
	  -ldflags "-X $(CLI_PKG).version=$(BASE_VERSION)-dev+$(COMMIT) -X $(ENV_PKG).BuildEnv=dev" \
	  $(CMD)

test:
	go test ./...

vet:
	go vet ./...

fmt:
	go fmt ./...

clean:
	rm -rf bin/ dist/

# ── Cross-compilation ─────────────────────────────────────────────────────────
# Builds binaries for all supported npm platforms into dist/.

PLATFORMS := \
  linux/amd64/linux-x64/ \
  linux/arm64/linux-arm64/ \
  darwin/amd64/darwin-x64/ \
  darwin/arm64/darwin-arm64/ \
  windows/amd64/win32-x64/

# build-all: cross-compile for all platforms.
# BUILD_ENV=dev|stg|prod  VERSION=<version> (required for stg/prod, auto for dev)
#   make build-all BUILD_ENV=dev
#   make build-all BUILD_ENV=stg VERSION=1.2.0-rc.1
#   make build-all BUILD_ENV=prod VERSION=1.2.0
_BENV := $(or $(BUILD_ENV),prod)
_BVER := $(if $(filter dev,$(_BENV)),$(BASE_VERSION)-dev+$(COMMIT),$(VERSION))
RELEASE_LDFLAGS := -X $(CLI_PKG).version=$(_BVER) -X $(ENV_PKG).BuildEnv=$(_BENV)

build-all:
	@[ "$(_BENV)" = "dev" ] || [ -n "$(VERSION)" ] \
	  || (echo "Error: VERSION required for BUILD_ENV=$(_BENV)  (e.g. make build-all BUILD_ENV=$(_BENV) VERSION=x.y.z)" && exit 1)
	@mkdir -p dist
	@$(foreach p,$(PLATFORMS), \
	  $(eval GOOS   := $(word 1,$(subst /, ,$(p)))) \
	  $(eval GOARCH := $(word 2,$(subst /, ,$(p)))) \
	  $(eval EXT    := $(if $(filter windows,$(GOOS)),.exe,)) \
	  echo "Building $(GOOS)/$(GOARCH) [$(_BENV) $(_BVER)]..." && \
	  GOOS=$(GOOS) GOARCH=$(GOARCH) go build \
	    -ldflags "$(RELEASE_LDFLAGS)" \
	    -o dist/$(BINARY)-$(GOOS)-$(GOARCH)$(EXT) \
	    $(CMD) && \
	)
	@echo "All binaries built in dist/"

# ── npm packaging ─────────────────────────────────────────────────────────────
# After running build-all, copy binaries into each platform package's bin/ dir.

npm-stage: build-all
	@$(foreach p,$(PLATFORMS), \
	  $(eval GOOS   := $(word 1,$(subst /, ,$(p)))) \
	  $(eval GOARCH := $(word 2,$(subst /, ,$(p)))) \
	  $(eval PKG    := $(word 3,$(subst /, ,$(p)))) \
	  $(eval EXT    := $(if $(filter windows,$(GOOS)),.exe,)) \
	  $(eval PKGDIR := npm/packages/@rover/agent-cli-$(PKG)) \
	  mkdir -p $(PKGDIR)/bin && \
	  cp dist/$(BINARY)-$(GOOS)-$(GOARCH)$(EXT) $(PKGDIR)/bin/agc$(EXT) && \
	  $(if $(filter-out windows,$(GOOS)),chmod +x $(PKGDIR)/bin/agc$(EXT),true) && \
	)
	@echo "Binaries staged into npm/packages/"

npm-pack: npm-stage
	@echo "Packing platform packages..."
	@cd npm && npm pack --pack-destination ../dist/
	@$(foreach p,$(PLATFORMS), \
	  $(eval PKG    := $(word 3,$(subst /, ,$(p)))) \
	  echo "Packing @rover/agent-cli-$(PKG)..." && \
	  cd npm/packages/@rover/agent-cli-$(PKG) && npm pack --pack-destination ../../../../dist/ && cd ../../../.. && \
	)
	@echo "Tarballs written to dist/"

npm-publish: npm-pack
	@echo "Publishing to npm..."
	@$(foreach p,$(PLATFORMS), \
	  $(eval PKG    := $(word 3,$(subst /, ,$(p)))) \
	  npm publish npm/packages/@rover/agent-cli-$(PKG) --access public && \
	)
	@npm publish npm/ --access public
	@echo "Published @rover/agent-cli $(VERSION)"

# ── Skill generation ──────────────────────────────────────────────────────────

generate-skills: build
	./bin/$(BINARY) generate-skills --output-dir skills/

# ── Version sync ──────────────────────────────────────────────────────────────
# Keep go.mod pseudo-version and npm package.json in sync (called by release workflow).

version-sync:
	@NEW_VERSION=$(v); \
	[ -z "$$NEW_VERSION" ] && echo "Usage: make version-sync v=1.2.3" && exit 1; \
	node -e "const f='npm/package.json'; const p=JSON.parse(require('fs').readFileSync(f)); p.version='$$NEW_VERSION'; require('fs').writeFileSync(f,JSON.stringify(p,null,2)+'\n')"; \
	$(foreach p,$(PLATFORMS), \
	  $(eval PKG := $(word 3,$(subst /, ,$(p)))) \
	  node -e "const f='npm/packages/@rover/agent-cli-$(PKG)/package.json'; const p=JSON.parse(require('fs').readFileSync(f)); p.version='$$NEW_VERSION'; require('fs').writeFileSync(f,JSON.stringify(p,null,2)+'\n')"; \
	) \
	sed -i 's/^var version = ".*"/var version = "'"$$NEW_VERSION"'" \/\/ overridden at build time via -ldflags/' internal/cli/root.go; \
	sed -i 's/^const skillVersion = ".*"/const skillVersion = "'"$$NEW_VERSION"'"/' internal/cli/generate_skills.go; \
	echo "Version bumped to $$NEW_VERSION"
