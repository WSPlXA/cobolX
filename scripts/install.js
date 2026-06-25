const fs = require('fs');
const path = require('path');
const https = require('https');
const { execSync } = require('child_process');

const pkg = require('../package.json');
const version = pkg.version;
const repo = 'WSPlXA/cobolX';
const binDir = path.join(__dirname, '../bin');

// Map process.platform and process.arch to Rust target triples and archives
function getTarget() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === 'win32' && arch === 'x64') {
    return {
      triple: 'x86_64-pc-windows-msvc',
      ext: 'zip'
    };
  }
  if (platform === 'darwin') {
    if (arch === 'x64') {
      return {
        triple: 'x86_64-apple-darwin',
        ext: 'tar.gz'
      };
    }
    if (arch === 'arm64') {
      return {
        triple: 'aarch64-apple-darwin',
        ext: 'tar.gz'
      };
    }
  }
  if (platform === 'linux') {
    if (arch === 'x64') {
      return {
        triple: 'x86_64-unknown-linux-gnu',
        ext: 'tar.gz'
      };
    }
    if (arch === 'arm64') {
      return {
        triple: 'aarch64-unknown-linux-gnu',
        ext: 'tar.gz'
      };
    }
  }

  throw new Error(`Unsupported platform/architecture: ${platform}/${arch}`);
}

function downloadFile(url, dest, callback) {
  https.get(url, (res) => {
    if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
      // Follow redirect
      downloadFile(res.headers.location, dest, callback);
    } else if (res.statusCode === 200) {
      const file = fs.createWriteStream(dest);
      res.pipe(file);
      file.on('finish', () => {
        file.close(callback);
      });
    } else {
      callback(new Error(`Failed to download: HTTP ${res.statusCode} ${res.statusMessage}`));
    }
  }).on('error', (err) => {
    callback(err);
  });
}

function install() {
  try {
    const target = getTarget();
    const archiveName = `rdo-${target.triple}.${target.ext}`;
    const downloadUrl = `https://github.com/${repo}/releases/download/v${version}/${archiveName}`;
    const tempFile = path.join(binDir, `temp-${archiveName}`);

    console.log(`Downloading CobolX binary for ${target.triple}...`);
    
    // Ensure bin directory exists
    if (!fs.existsSync(binDir)) {
      fs.mkdirSync(binDir, { recursive: true });
    }

    downloadFile(downloadUrl, tempFile, (err) => {
      if (err) {
        console.error('Error downloading binary:', err.message);
        process.exit(1);
      }

      console.log('Extracting binary...');
      try {
        if (target.ext === 'zip') {
          // Decompress zip on Windows using PowerShell
          const cmd = `powershell -Command "Expand-Archive -Path '${tempFile}' -DestinationPath '${binDir}' -Force"`;
          execSync(cmd, { stdio: 'inherit' });
        } else {
          // Decompress tar.gz on Unix
          const cmd = `tar -xzf "${tempFile}" -C "${binDir}"`;
          execSync(cmd, { stdio: 'inherit' });
        }

        // Clean up temp archive
        fs.unlinkSync(tempFile);

        // Rename the binary from 'rdo' to 'cobolx'
        const rawBinName = target.ext === 'zip' ? 'rdo.exe' : 'rdo';
        const finalBinName = target.ext === 'zip' ? 'cobolx.exe' : 'cobolx';

        const rawBinPath = path.join(binDir, rawBinName);
        const finalBinPath = path.join(binDir, finalBinName);

        if (fs.existsSync(rawBinPath)) {
          fs.renameSync(rawBinPath, finalBinPath);
        } else {
          // Check if it was already named correctly or packaged differently
          if (!fs.existsSync(finalBinPath)) {
            throw new Error(`Could not find extracted binary in ${binDir}`);
          }
        }

        // Set execution permissions on Unix
        if (target.ext !== 'zip') {
          fs.chmodSync(finalBinPath, 0o755);
        }

        console.log('CobolX binary installed successfully.');
      } catch (extractErr) {
        console.error('Error extracting binary:', extractErr.message);
        // Clean up temp file in case of failure
        if (fs.existsSync(tempFile)) {
          fs.unlinkSync(tempFile);
        }
        process.exit(1);
      }
    });
  } catch (err) {
    console.error('Installation failed:', err.message);
    process.exit(1);
  }
}

install();
