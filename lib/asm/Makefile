# sBPF Assembly Build for HyperTree Optimizations
#
# Prerequisites:
#   - sbpf SDK installed: cargo install --git https://github.com/blueshift-gg/sbpf.git
#
# Usage:
#   make        - Build all assembly files
#   make clean  - Remove build artifacts
#   make test   - Run tests

SBPF = sbpf
ASM_DIR = src
BUILD_DIR = build
OUT_FILE = $(BUILD_DIR)/hypertree_asm.so

ASM_FILES = $(wildcard $(ASM_DIR)/*.s)
OBJ_FILES = $(patsubst $(ASM_DIR)/%.s,$(BUILD_DIR)/%.o,$(ASM_FILES))

.PHONY: all clean test

all: $(OUT_FILE)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/%.o: $(ASM_DIR)/%.s | $(BUILD_DIR)
	$(SBPF) assemble $< -o $@

$(OUT_FILE): $(OBJ_FILES)
	$(SBPF) link $(OBJ_FILES) -o $@

clean:
	rm -rf $(BUILD_DIR)

test:
	@echo "Running sBPF assembly tests..."
	@for test in tests/*.s; do \
		echo "Testing $$test..."; \
		$(SBPF) test $$test; \
	done
