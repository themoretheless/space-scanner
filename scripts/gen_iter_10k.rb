#!/usr/bin/env ruby
# frozen_string_literal: true

# Next 10k unique rules from leftover catalog names × 3 OS.
# Usage: ruby scripts/gen_iter_10k.rb [wave] [nix|debian|aur]
# Writes rules/generated/src-iter{wave}-*.yaml only if exactly TARGET rules.

require "yaml"
require "set"
require "fileutils"
require "zlib"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
WAVE = Integer(ARGV[0] || ENV.fetch("WAVE", "4"))
CATALOG = (ARGV[1] || ENV.fetch("CATALOG", "nix")).downcase
TARGET = 10_000
NAMES, PREFIX, LABEL = case CATALOG
                       when "debian"
                         ["/tmp/debian-names.txt", "db#{WAVE}", "Debian"]
                       when "aur"
                         ["/tmp/aur-packages.gz", "au#{WAVE}", "AUR"]
                       else
                         ["/tmp/nix-pnames.txt", "np#{WAVE}", "Nixpkgs"]
                       end

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 64]
end

def safe_pkg?(name)
  return false if name.nil? || name.empty?
  return false if name.length < 2 || name.length > 64
  return false if name.include?("/") || name.include?("\\") || name.include?(" ")
  name.match?(/\A[a-zA-Z][a-zA-Z0-9+._-]*\z/)
end

def each_name(path)
  if path.end_with?(".gz")
    Zlib::GzipReader.open(path) do |gz|
      gz.each_line { |line| yield line }
    end
  else
    File.foreach(path) { |line| yield line }
  end
end

seen = Set.new
used_slugs = Set.new
Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
  next if File.basename(f).start_with?("_")
  File.read(f).scan(/^- id: (.+)$/) do |m|
    id = m[0].strip
    seen << id
    if (mm = id.match(/\A(?:nix|np\d+|db\d+|au\d+)-mac-(.+)\z/))
      used_slugs << mm[1]
    end
  end
end

abort "missing #{NAMES}" unless File.exist?(NAMES)

rules = []
each_name(NAMES) do |line|
  break if rules.size >= TARGET
  pkg = line.strip
  next unless safe_pkg?(pkg)
  s = slug(pkg)
  next if s.length < 2 || used_slugs.include?(s)
  next if seen.include?("#{PREFIX}-mac-#{s}")
  [
    ["#{PREFIX}-mac-#{s}", "macos", ["$HOME/Library/Caches/#{pkg}", "$HOME/Library/Logs/#{pkg}"]],
    ["#{PREFIX}-lin-#{s}", "linux", ["$HOME/.cache/#{pkg}", "$XDG_CACHE_HOME/#{pkg}"]],
    ["#{PREFIX}-win-#{s}", "windows", ["%LOCALAPPDATA%/#{pkg}/Cache", "%LOCALAPPDATA%/#{pkg}/Logs"]]
  ].each do |id, plat, paths|
    next if seen.include?(id)
    next if rules.size >= TARGET
    seen << id
    used_slugs << s if plat == "macos"
    rules << {
      "id" => id,
      "name" => "#{LABEL} #{pkg} #{plat} cache (wave #{WAVE})",
      "description" => "#{LABEL} leftover name #{pkg} on #{plat} (wave #{WAVE}). Caches/logs only, not configs.",
      "category" => "caches",
      "platform" => plat,
      "risk" => "low",
      "default_enabled" => false,
      "min_age_days" => 14,
      "paths" => paths,
      "delete_mode" => "contents",
      "max_depth" => 6
    }
  end
end

tag = format("src-iter%02d", WAVE)
Dir.glob(File.join(OUT, "#{tag}-*.yaml")).each { |f| File.delete(f) }
if rules.size < TARGET
  warn "wave #{WAVE} #{CATALOG}: only #{rules.size} leftover names (need #{TARGET}); not writing"
  exit 2
end

FileUtils.mkdir_p(OUT)
rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  File.write(File.join(OUT, format("#{tag}-%02d.yaml", i)), YAML.dump(
    "pack" => "#{tag}-#{i}",
    "description" => "Iterative 10k import wave #{WAVE} from leftover #{LABEL} names. Chunk #{i}.",
    "rules" => chunk
  ))
end

mac = rules.count { |r| r["platform"] == "macos" }
lin = rules.count { |r| r["platform"] == "linux" }
win = rules.count { |r| r["platform"] == "windows" }
puts "wave=#{WAVE} catalog=#{CATALOG} wrote=#{rules.size} macos=#{mac} linux=#{lin} windows=#{win} used_slugs=#{used_slugs.size}"
