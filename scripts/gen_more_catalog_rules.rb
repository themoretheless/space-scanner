#!/usr/bin/env ruby
# frozen_string_literal: true

# Another unique wave: leftover Nixpkgs pnames × 3 OS, MacPorts, Chocolatey, leftover AUR.
# Writes rules/generated/src-more-*.yaml

require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 64]
end

def safe_pkg?(name)
  return false if name.nil? || name.empty?
  return false if name.length < 2 || name.length > 64
  return false if name.include?("/") || name.include?("\\") || name.include?(" ")
  name.match?(/\A[a-zA-Z][a-zA-Z0-9+._-]*\z/)
end

seen = Set.new
Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
  next if File.basename(f).start_with?("_")
  next if File.basename(f).start_with?("src-more-")
  File.read(f).scan(/^- id: (.+)$/) { |m| seen << m[0].strip }
end

used_nix = Set.new
seen.each { |id| used_nix << id.delete_prefix("nix-mac-") if id.start_with?("nix-mac-") }

rules = []
add = lambda do |id, name, desc, cat, plat, paths|
  return false if seen.include?(id)
  seen << id
  rules << {
    "id" => id,
    "name" => name.to_s[0, 120],
    "description" => desc.to_s[0, 400],
    "category" => cat,
    "platform" => plat,
    "risk" => "low",
    "default_enabled" => false,
    "min_age_days" => 14,
    "paths" => Array(paths),
    "delete_mode" => "contents",
    "max_depth" => 6
  }
  true
end

# Remaining Nixpkgs pnames (skip those already in nix-mac-*)
if File.exist?("/tmp/nix-pnames.txt")
  n = 0
  File.foreach("/tmp/nix-pnames.txt") do |line|
    break if n >= 5000
    pkg = line.strip
    next unless safe_pkg?(pkg)
    s = slug(pkg)
    next if s.length < 2 || used_nix.include?(s)
    next if seen.include?("np2-mac-#{s}")
    add.call("np2-mac-#{s}", "Nixpkgs #{pkg} macOS cache (wave 2)",
      "Second Nixpkgs slice for #{pkg} on macOS. Not /nix/store.", "caches", "macos",
      ["$HOME/Library/Caches/#{pkg}", "$HOME/Library/Logs/#{pkg}"])
    add.call("np2-lin-#{s}", "Nixpkgs #{pkg} Linux cache (wave 2)",
      "Second Nixpkgs slice for #{pkg} on Linux. Not /nix/store.", "caches", "linux",
      ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}"])
    add.call("np2-win-#{s}", "Nixpkgs #{pkg} Windows cache (wave 2)",
      "Second Nixpkgs slice for #{pkg} on Windows.", "caches", "windows",
      ["%LOCALAPPDATA%/#{pkg}/Cache", "%LOCALAPPDATA%/#{pkg}/Logs"])
    n += 1
  end
end

# MacPorts
if File.exist?("/tmp/macports-names.txt")
  File.foreach("/tmp/macports-names.txt") do |line|
    pkg = line.strip
    next unless safe_pkg?(pkg)
    s = slug(pkg)
    next if s.length < 2
    add.call("mp-#{s}", "MacPorts #{pkg} cache",
      "Home cache/logs for MacPorts port #{pkg} (ports.macports.org). Not /opt/local prefix.",
      "caches", "macos",
      ["$HOME/Library/Caches/#{pkg}", "$HOME/Library/Logs/#{pkg}", "$HOME/.cache/#{pkg}"])
  end
end

# Chocolatey
if File.exist?("/tmp/choco-ids.txt") && File.size("/tmp/choco-ids.txt") > 0
  File.foreach("/tmp/choco-ids.txt") do |line|
    pkg = line.strip
    next unless safe_pkg?(pkg)
    s = slug(pkg)
    next if s.length < 2
    add.call("choco-#{s}", "Chocolatey #{pkg} cache",
      "Windows cache/logs for Chocolatey package #{pkg}. Not lib of installed packages.",
      "app-cache", "windows",
      ["%LOCALAPPDATA%/#{pkg}/Cache", "%APPDATA%/#{pkg}/Cache", "%PROGRAMDATA%/chocolatey/lib/#{pkg}/tools/cache"])
  end
end

# Leftover AUR (linux) after first 5500
if File.exist?("/tmp/aur-packages.gz")
  skip = 0
  added = 0
  IO.popen(["gzip", "-dc", "/tmp/aur-packages.gz"]) do |io|
    io.each_line do |line|
      skip += 1
      next if skip <= 5500
      break if added >= 4000
      pkg = line.strip
      next unless safe_pkg?(pkg)
      s = slug(pkg)
      next if s.length < 2
      if add.call("aur2-#{s}", "AUR #{pkg} logs",
        "User logs/cache leftover for AUR package #{pkg} (second slice).",
        "logs", "linux",
        ["$HOME/.cache/#{pkg}/logs", "$HOME/.local/share/#{pkg}/logs", "$XDG_CACHE_HOME/#{pkg}/logs"])
        added += 1
      end
    end
  end
end

FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src-more-*.yaml")).each { |f| File.delete(f) }
rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  File.write(File.join(OUT, format("src-more-%02d.yaml", i)), YAML.dump(
    "pack" => "src-more-#{i}",
    "description" => "Further catalog import: Nixpkgs wave 2, MacPorts, Chocolatey, AUR slice 2. Chunk #{i}.",
    "rules" => chunk
  ))
end

puts "wrote #{rules.size} macos=#{rules.count { |r| r['platform']=='macos' }} linux=#{rules.count { |r| r['platform']=='linux' }} windows=#{rules.count { |r| r['platform']=='windows' }}"
