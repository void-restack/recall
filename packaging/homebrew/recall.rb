# Homebrew formula for recall.
#
# This installs the prebuilt binaries published by the release workflow. It is meant
# to live in a tap repo (e.g. void-restack/homebrew-recall as Formula/recall.rb).
# After each release, update `version` and the four `sha256` values from the
# `*.sha256` files attached to the GitHub Release.
class Recall < Formula
  desc "Fast, local command memory for your terminal"
  homepage "https://github.com/void-restack/recall"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/void-restack/recall/releases/download/v#{version}/recall-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
    on_intel do
      url "https://github.com/void-restack/recall/releases/download/v#{version}/recall-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/void-restack/recall/releases/download/v#{version}/recall-v#{version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
    on_intel do
      url "https://github.com/void-restack/recall/releases/download/v#{version}/recall-v#{version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
  end

  def install
    bin.install "recall"
  end

  test do
    assert_match "recall", shell_output("#{bin}/recall --version")
  end
end
