# NOTE: this file is a mirror.
# The canonical formula lives at https://github.com/matagus/homebrew-tap/blob/main/Formula/shelve.rb
# (brew install matagus/tap/shelve). It is kept here only as a reference for
# source builds and is not wired into any release automation.
class Shelve < Formula
  desc "Pretty-print CSV files grouped by a column"
  homepage "https://github.com/matagus/shelve"
  url "https://github.com/matagus/shelve/archive/refs/tags/v0.4.1.tar.gz"
  sha256 "2a1a3f4dcf03f8008b1f46ad172440baf0443bdeaced2e5c89154765ff5c3b8c"
  license "MIT"
  head "https://github.com/matagus/shelve.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    (testpath/"test.csv").write <<~CSV
      name,status
      alice,active
      bob,inactive
      carol,active
    CSV

    output = shell_output("#{bin}/shelve -c 2 #{testpath}/test.csv")
    assert_match "active", output
    assert_match "inactive", output
  end
end
