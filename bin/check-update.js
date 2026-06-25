const fs = require('fs');
const path = require('path');
const https = require('https');

const pkg = require('../package.json');
const noticeFile = path.join(__dirname, '.update-notice');

const options = {
  hostname: 'registry.npmjs.org',
  path: `/${pkg.name}/latest`,
  timeout: 3000,
  headers: {
    'User-Agent': 'cobolx-cli-update-checker'
  }
};

const req = https.get(options, (res) => {
  if (res.statusCode !== 200) return;
  let data = '';
  res.on('data', (chunk) => { data += chunk; });
  res.on('end', () => {
    try {
      const info = JSON.parse(data);
      const latest = info.version;
      if (latest && latest !== pkg.version) {
        const msg = [
          '\x1b[33m---------------------------------------------------------\x1b[0m',
          `  💡 \x1b[36mUpdate available:\x1b[0m \x1b[32m${latest}\x1b[0m (current: ${pkg.version})`,
          `     Run \x1b[33mnpm update -g ${pkg.name}\x1b[0m to update!`,
          '\x1b[33m---------------------------------------------------------\x1b[0m'
        ].join('\n');
        fs.writeFileSync(noticeFile, msg, 'utf8');
      }
    } catch (e) {
      // ignore
    }
  });
});

req.on('error', () => {
  // ignore
});

req.end();
