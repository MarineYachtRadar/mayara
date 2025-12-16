#!/usr/bin/env node
/**
 * Cross-platform build script for mayara-signalk-wasm
 *
 * Usage: node build.js [--test] [--no-pack]
 *   --test     Run cargo tests before building
 *   --no-pack  Skip creating npm package (default: creates package)
 */

const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const args = process.argv.slice(2);
const runTests = args.includes('--test');
const skipPack = args.includes('--no-pack');

const WASM_TARGET = 'wasm32-wasip1';
const CRATE_NAME = 'mayara_signalk_wasm';

// Paths (relative to this script's directory)
const scriptDir = __dirname;
const projectRoot = path.resolve(scriptDir, '..');
const wasmSource = path.join(projectRoot, 'target', WASM_TARGET, 'release', `${CRATE_NAME}.wasm`);
const wasmDest = path.join(scriptDir, 'plugin.wasm');
const guiSource = path.join(projectRoot, 'mayara-gui');
const publicDest = path.join(scriptDir, 'public');

function run(cmd, options = {}) {
  console.log(`> ${cmd}`);
  try {
    execSync(cmd, { stdio: 'inherit', cwd: options.cwd || projectRoot, ...options });
  } catch (e) {
    console.error(`Command failed: ${cmd}`);
    process.exit(1);
  }
}

/**
 * Recursively copy directory contents
 */
function copyDir(src, dest) {
  if (!fs.existsSync(src)) {
    console.error(`Source directory not found: ${src}`);
    process.exit(1);
  }

  // Remove destination if it exists
  if (fs.existsSync(dest)) {
    fs.rmSync(dest, { recursive: true });
  }

  // Create destination directory
  fs.mkdirSync(dest, { recursive: true });

  // Copy all files and subdirectories
  const entries = fs.readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDir(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

function main() {
  console.log('=== Mayara SignalK WASM Build ===\n');

  // Step 1: Run tests (optional)
  if (runTests) {
    console.log('Step 1: Running tests...\n');
    run('cargo test -p mayara-core');
    console.log('\n');
  }

  // Step 2: Copy GUI assets from shared mayara-gui
  console.log('Step 2: Copying GUI assets from mayara-gui...\n');
  copyDir(guiSource, publicDest);
  const fileCount = fs.readdirSync(publicDest, { recursive: true }).length;
  console.log(`Copied ${fileCount} files from mayara-gui/ to public/\n`);

  // Step 3: Build WASM
  console.log('Step 3: Building WASM...\n');
  run(`cargo build --target ${WASM_TARGET} --release -p mayara-signalk-wasm`);
  console.log('\n');

  // Step 4: Copy WASM file
  console.log('Step 4: Copying WASM file...\n');
  if (!fs.existsSync(wasmSource)) {
    console.error(`WASM file not found: ${wasmSource}`);
    process.exit(1);
  }
  fs.copyFileSync(wasmSource, wasmDest);
  const size = fs.statSync(wasmDest).size;
  console.log(`Copied ${wasmSource} -> plugin.wasm (${(size / 1024).toFixed(1)} KB)\n`);

  // Step 5: Pack (unless --no-pack)
  if (!skipPack) {
    console.log('Step 5: Creating npm package...\n');
    run('npm pack', { cwd: scriptDir });
    console.log('\n');
  }

  console.log('=== Build complete ===');
}

main();
