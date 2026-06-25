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

// 1. Check for pending update notification from previous background checks
const noticeFile = path.join(__dirname, '.update-notice');
if (fs.existsSync(noticeFile)) {
  try {
    const notice = fs.readFileSync(noticeFile, 'utf8').trim();
    if (notice) {
      console.log('\n' + notice);
    }
    fs.unlinkSync(noticeFile);
  } catch (err) {
    // ignore
  }
}

// 2. Start the native binary child process
const args = process.argv.slice(2);
const child = spawn(binaryPath, args, { stdio: 'inherit' });

child.on('error', (err) => {
  console.error('Failed to start the native binary:', err);
  process.exit(1);
});

child.on('close', (code) => {
  // 3. Trigger background update check
  triggerBackgroundUpdateCheck();
  process.exit(code ?? 0);
});

function triggerBackgroundUpdateCheck() {
  try {
    const pkg = require('../package.json');
    const cacheFile = path.join(__dirname, '.last-update-check');
    const CHECK_INTERVAL = 24 * 60 * 60 * 1000; // 24 hours
    const now = Date.now();
    
    let lastCheck = 0;
    if (fs.existsSync(cacheFile)) {
      lastCheck = parseInt(fs.readFileSync(cacheFile, 'utf8'), 10) || 0;
    }

    if (now - lastCheck < CHECK_INTERVAL) {
      return;
    }

    // Write new timestamp immediately to throttle requests
    fs.writeFileSync(cacheFile, now.toString(), 'utf8');

    // Spawn a detached background process to check for updates
    const checkerScript = path.join(__dirname, 'check-update.js');
    
    // Ignore outputs or pipe to a log file
    const logFile = path.join(__dirname, '.checker-log');
    const out = fs.openSync(logFile, 'a');
    
    const child = spawn(process.execPath, [checkerScript], {
      detached: true,
      stdio: ['ignore', out, out]
    });
    
    child.unref();
  } catch (err) {
    // ignore
  }
}
