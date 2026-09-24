class Shelve < Formula
  desc "Pretty-print CSV files grouped by a column"
  homepage "https://github.com/matagus/shelve"
  url "https://github.com/matagus/shelve/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "PLACEHOLDER_SHA256"
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
