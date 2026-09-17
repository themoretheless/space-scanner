#!/usr/bin/env ruby
# frozen_string_literal: true

# 30k unique rules: leftover Nixpkgs pnames × macOS/Linux/Windows.
# Writes rules/generated/src30k-*.yaml

require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
PER_OS = 10_000
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
used_nix = Set.new
Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
  next if File.basename(f).start_with?("_")
  next if File.basename(f).start_with?("src30k-")
  File.read(f).scan(/^- id: (.+)$/) do |m|
    id = m[0].strip
    seen << id
    if id.start_with?("nix-mac-")
      used_nix << id.delete_prefix("nix-mac-")
    elsif id.start_with?("np2-mac-")
      used_nix << id.delete_prefix("np2-mac-")
    end
  end
end

abort "missing #{NAMES}" unless File.exist?(NAMES)

rules = []
add = lambda do |id, name, desc, plat, paths|
  return false if seen.include?(id)
  seen << id
  rules << {
    "id" => id,
    "name" => name[0, 120],
    "description" => desc[0, 400],
    "category" => "caches",
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
File.foreach(NAMES) do |line|
  break if picked >= PER_OS
  pkg = line.strip
  next unless safe_pkg?(pkg)
  s = slug(pkg)
  next if s.length < 2 || used_nix.include?(s)
  next if seen.include?("np3-mac-#{s}")
  ok = add.call("np3-mac-#{s}", "Nixpkgs #{pkg} macOS cache (wave 3)",
    "Third Nixpkgs slice for #{pkg} on macOS. Not /nix/store.", "macos",
    ["$HOME/Library/Caches/#{pkg}", "$HOME/Library/Logs/#{pkg}"])
  ok &&= add.call("np3-lin-#{s}", "Nixpkgs #{pkg} Linux cache (wave 3)",
    "Third Nixpkgs slice for #{pkg} on Linux. Not /nix/store.", "linux",
    ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}"])
  ok &&= add.call("np3-win-#{s}", "Nixpkgs #{pkg} Windows cache (wave 3)",
    "Third Nixpkgs slice for #{pkg} on Windows.", "windows",
    ["%LOCALAPPDATA%/#{pkg}/Cache", "%LOCALAPPDATA%/#{pkg}/Logs"])
  picked += 1 if ok
end

warn "only #{rules.size} (want #{PER_OS * 3})" if rules.size < PER_OS * 3

FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src30k-*.yaml")).each { |f| File.delete(f) }
rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  File.write(File.join(OUT, format("src30k-%02d.yaml", i)), YAML.dump(
    "pack" => "src30k-#{i}",
    "description" => "Nixpkgs wave 3 × macOS/Linux/Windows. Chunk #{i}.",
    "rules" => chunk
  ))
end

puts "wrote #{rules.size} macos=#{rules.count { |r| r['platform']=='macos' }} linux=#{rules.count { |r| r['platform']=='linux' }} windows=#{rules.count { |r| r['platform']=='windows' }} picked=#{picked}"
