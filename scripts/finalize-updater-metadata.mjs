#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';

const version = process.env.VERSION || process.argv[2] || '';
const repository = process.env.GITHUB_REPOSITORY || process.argv[3] || '';

if (!version.startsWith('v')) {
  throw new Error('VERSION must be a tag like v0.0.9');
}
if (!repository || !repository.includes('/')) {
  throw new Error('GITHUB_REPOSITORY must be set, for example anYuJia/douyin-downloader-rust');
}

const appVersion = version.slice(1);
const baseUrl = `https://github.com/${repository}/releases/download/${version}`;
const previous = fs.existsSync('latest.json')
  ? JSON.parse(fs.readFileSync('latest.json', 'utf8'))
  : {};

function findFile(pattern, label) {
  const found = fs.readdirSync(process.cwd()).find((file) => pattern.test(file));
  if (!found) {
    throw new Error(`Missing ${label}`);
  }
  return found;
}

function readSignature(assetName) {
  const sigPath = `${assetName}.sig`;
  if (!fs.existsSync(sigPath)) {
    throw new Error(`Missing signature file: ${sigPath}`);
  }
  return fs.readFileSync(sigPath, 'utf8').replace(/[\r\n]/g, '');
}

const assets = {
  darwinArmApp: 'Douyin.Downloader_aarch64.app.tar.gz',
  darwinX64App: 'Douyin.Downloader_x64.app.tar.gz',
  windowsInstaller: `Douyin.Downloader_${appVersion}_x64-setup.exe`,
  windowsPortable: `Douyin-Downloader_${appVersion}_x64_portable.exe`,
  linuxAppImage: `Douyin.Downloader_${appVersion}_amd64.AppImage`,
  linuxDeb: `Douyin.Downloader_${appVersion}_amd64.deb`,
  linuxRpm: findFile(
    new RegExp(`^Douyin\\.Downloader-${appVersion.replaceAll('.', '\\.')}-\\d+\\.x86_64\\.rpm\\.sig$`),
    'RPM signature'
  ).replace(/\.sig$/, '')
};

const metadata = {
  version: appVersion,
  notes: previous.notes || '',
  pub_date: previous.pub_date || '',
  platforms: {
    'darwin-aarch64': {
      signature: readSignature(assets.darwinArmApp),
      url: `${baseUrl}/${assets.darwinArmApp}`
    },
    'darwin-aarch64-app': {
      signature: readSignature(assets.darwinArmApp),
      url: `${baseUrl}/${assets.darwinArmApp}`
    },
    'darwin-x86_64': {
      signature: readSignature(assets.darwinX64App),
      url: `${baseUrl}/${assets.darwinX64App}`
    },
    'darwin-x86_64-app': {
      signature: readSignature(assets.darwinX64App),
      url: `${baseUrl}/${assets.darwinX64App}`
    },
    'windows-x86_64': {
      signature: readSignature(assets.windowsInstaller),
      url: `${baseUrl}/${assets.windowsInstaller}`
    },
    'windows-x86_64-nsis': {
      signature: readSignature(assets.windowsInstaller),
      url: `${baseUrl}/${assets.windowsInstaller}`
    },
    'windows-x86_64-portable': {
      signature: readSignature(assets.windowsPortable),
      url: `${baseUrl}/${assets.windowsPortable}`
    },
    'linux-x86_64': {
      signature: readSignature(assets.linuxAppImage),
      url: `${baseUrl}/${assets.linuxAppImage}`
    },
    'linux-x86_64-appimage': {
      signature: readSignature(assets.linuxAppImage),
      url: `${baseUrl}/${assets.linuxAppImage}`
    },
    'linux-x86_64-deb': {
      signature: readSignature(assets.linuxDeb),
      url: `${baseUrl}/${assets.linuxDeb}`
    },
    'linux-x86_64-rpm': {
      signature: readSignature(assets.linuxRpm),
      url: `${baseUrl}/${assets.linuxRpm}`
    }
  }
};

for (const [target, data] of Object.entries(metadata.platforms)) {
  if (!data.signature || !data.url) {
    throw new Error(`Incomplete updater metadata for ${target}`);
  }
}

for (const output of ['latest.json', 'darwin.json', 'windows.json', 'linux.json']) {
  fs.writeFileSync(path.join(process.cwd(), output), `${JSON.stringify(metadata, null, 2)}\n`);
}

console.log(`Generated updater metadata for ${version}`);
