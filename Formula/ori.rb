# frozen_string_literal: true

# Homebrew formula for the Orison CLI (`ori`).
#
# This formula installs a prebuilt binary from the GitHub Release matching the
# version below. The `sha256` values are placeholders for the first release and
# MUST be replaced with the real digests printed by the `release-publish`
# workflow when cutting v0.1.1 (or whatever the first published tag is).
#
# To install from a tap:
#   brew install Eldergenix/orison/ori
#
# To install directly from this file:
#   brew install --formula ./Formula/ori.rb
class Ori < Formula
  desc "Orison language toolchain CLI"
  homepage "https://github.com/Eldergenix/Orison"
  version "0.1.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Eldergenix/Orison/releases/download/v0.1.1/ori-macos-aarch64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/Eldergenix/Orison/releases/download/v0.1.1/ori-macos-x86_64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Eldergenix/Orison/releases/download/v0.1.1/ori-linux-aarch64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/Eldergenix/Orison/releases/download/v0.1.1/ori-linux-x86_64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "ori"
    # The release archive ships LICENSE and README.md alongside the binary.
    pkgshare.install "LICENSE" if File.exist?("LICENSE")
    pkgshare.install "README.md" if File.exist?("README.md")
  end

  test do
    assert_match "ori", shell_output("#{bin}/ori --help 2>&1", 0..2)
  end
end
