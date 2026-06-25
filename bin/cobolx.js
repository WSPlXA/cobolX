#!/usr/bin/env node

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const isWin = process.platform === 'win32';
const binName = isWin ? 'cobolx.exe' : 'cobolx';
const binaryPath = path.join(__dirname, binName);

if (!fs.existsSync(binaryPath)) {
  console.error(`Error: Native binary not found at ${binaryPath}`);
  console.error('Please try reinstalling the package: npm install -g cobolx');
  process.exit(1);
}

const args = process.argv.slice(2);
const child = spawn(binaryPath, args, { stdio: 'inherit' });

child.on('error', (err) => {
  console.error('Failed to start the native binary:', err);
  process.exit(1);
});

child.on('close', (code) => {
  process.exit(code ?? 0);
});
