const fs = require('fs');
const path = require('path');

const githubRef = process.env.GITHUB_REF || '';
const runNumber = process.env.GITHUB_RUN_NUMBER || '0';
const runAttempt = process.env.GITHUB_RUN_ATTEMPT || '1';
const githubEnv = process.env.GITHUB_ENV;

const pkgPath = path.join(__dirname, '../package.json');
const cargoPath = path.join(__dirname, '../Cargo.toml');

const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
const baseVersion = pkg.version;

let version = baseVersion;
let isCanary = 'false';
let npmTag = 'latest';

if (githubRef.startsWith('refs/tags/v')) {
  version = githubRef.replace('refs/tags/v', '');
} else {
  // Use UTC YYYYMMDDHHMM timestamp to ensure uniqueness and prevent registry publish collisions
  const now = new Date();
  const timestamp = now.getUTCFullYear().toString() +
    String(now.getUTCMonth() + 1).padStart(2, '0') +
    String(now.getUTCDate()).padStart(2, '0') +
    String(now.getUTCHours()).padStart(2, '0') +
    String(now.getUTCMinutes()).padStart(2, '0');
  version = `${baseVersion}-canary.${runNumber}.${timestamp}`;
  isCanary = 'true';
  npmTag = 'canary';
}

console.log(`Setting version to: ${version}`);
console.log(`Is Canary: ${isCanary}`);
console.log(`NPM Tag: ${npmTag}`);

// Update package.json
pkg.version = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n', 'utf8');

// Update Cargo.toml
let cargo = fs.readFileSync(cargoPath, 'utf8');
cargo = cargo.replace(/version\s*=\s*"[^"]+"/, `version = "${version}"`);
fs.writeFileSync(cargoPath, cargo, 'utf8');

// Write to GITHUB_ENV
if (githubEnv && fs.existsSync(githubEnv)) {
  fs.appendFileSync(githubEnv, `RELEASE_VERSION=${version}\nIS_CANARY=${isCanary}\nNPM_TAG=${npmTag}\n`, 'utf8');
}
