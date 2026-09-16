#!/usr/bin/env ruby
# frozen_string_literal: true

# 15k unique rules split across macOS / Linux / Windows from Nixpkgs pnames.
# https://channels.nixos.org/nixos-unstable/packages.json.br
# Writes rules/generated/src15k-*.yaml

require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
PER_OS = 5_000
NAMES = "/tmp/nix-pnames.txt"

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
  next if File.basename(f).start_with?("src15k-")
  File.read(f).scan(/^- id: (.+)$/) { |m| seen << m[0].strip }
end

abort "missing #{NAMES}" unless File.exist?(NAMES)

names = File.foreach(NAMES).map(&:strip).select { |n| safe_pkg?(n) }.uniq
# Prefer letter-starting real package tokens; skip very generic
names.reject! { |n| n.match?(/\A(test|example|dummy|foo|bar)\z/i) }

rules = []
add = lambda do |id, name, desc, cat, plat, paths|
  return false if seen.include?(id)
  seen << id
  rules << {
    "id" => id,
    "name" => name[0, 120],
    "description" => desc[0, 400],
    "category" => cat,
    "platform" => plat,
    "risk" => "low",
    "default_enabled" => false,
    "min_age_days" => 14,
    "paths" => paths,
    "delete_mode" => "contents",
    "max_depth" => 6
  }
  true
end

picked = 0
names.each do |pkg|
  break if picked >= PER_OS
  s = slug(pkg)
  next if s.empty? || s.length < 2
  mac_id = "nix-mac-#{s}"
  lin_id = "nix-lin-#{s}"
  win_id = "nix-win-#{s}"
  next if seen.include?(mac_id) || seen.include?(lin_id) || seen.include?(win_id)

  ok = true
  ok &&= add.call(
    mac_id, "Nixpkgs #{pkg} macOS cache",
    "User cache/logs for Nixpkgs package #{pkg} (nixos-unstable packages.json). Not store paths or configs.",
    "caches", "macos",
    ["$HOME/Library/Caches/#{pkg}", "$HOME/Library/Logs/#{pkg}", "$HOME/.cache/#{pkg}"]
  )
  ok &&= add.call(
    lin_id, "Nixpkgs #{pkg} Linux cache",
    "XDG cache for Nixpkgs package #{pkg} (nixos-unstable packages.json). Not /nix/store.",
    "caches", "linux",
    ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}", "$HOME/.local/share/#{pkg}/cache"]
  )
  ok &&= add.call(
    win_id, "Nixpkgs #{pkg} Windows cache",
    "AppData cache for Nixpkgs package #{pkg} (used on Windows via nix). Not user documents.",
    "caches", "windows",
    ["%LOCALAPPDATA%/#{pkg}/Cache", "%APPDATA%/#{pkg}/Cache", "%LOCALAPPDATA%/#{pkg}/logs", "%TEMP%/#{pkg}"]
  )
  picked += 1 if ok
end

if rules.size < PER_OS * 3
  warn "only #{rules.size} rules (want #{PER_OS * 3}); picked=#{picked}"
end

FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src15k-*.yaml")).each { |f| File.delete(f) }
rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  File.write(File.join(OUT, format("src15k-%02d.yaml", i)), YAML.dump(
    "pack" => "src15k-#{i}",
    "description" => "Nixpkgs catalog split across macOS/Linux/Windows. Chunk #{i}.",
    "rules" => chunk
  ))
end

mac = rules.count { |r| r["platform"] == "macos" }
lin = rules.count { |r| r["platform"] == "linux" }
win = rules.count { |r| r["platform"] == "windows" }
puts "wrote #{rules.size} macos=#{mac} linux=#{lin} windows=#{win}"
