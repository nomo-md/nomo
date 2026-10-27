"""发布前校验六类发行资产，生成兼容下载站的 checksums.md5。"""
from pathlib import Path
import argparse
import hashlib
import re
import tarfile
import tomllib
import zipfile

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('directory', type=Path)
parser.add_argument('--tag', required=True)
args = parser.parse_args()
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
if args.tag != f'v{version}':
    raise SystemExit(f'Tag {args.tag!r} does not match Cargo version {version!r}')
expected = {
    f'Nomo_{version}_x64-setup.exe', f'Nomo_{version}_x64.zip',
    f'Nomo_{version}_aarch64.dmg', 'Nomo_aarch64.app.tar.gz',
    f'Nomo_{version}_amd64.deb', f'Nomo-{version}-1.x86_64.rpm',
}
files = {path.name for path in args.directory.iterdir() if path.is_file()}
if files != expected:
    raise SystemExit(f'Invalid release assets: missing={expected - files}, extra={files - expected}')
for name in expected:
    if (args.directory / name).stat().st_size == 0:
        raise SystemExit(f'Empty asset: {name}')
def validate_paths(names):
    for name in names:
        parts = name.replace('\\', '/').split('/')
        if name.startswith(('/', '\\')) or '..' in parts:
            raise SystemExit(f'Unsafe archive path: {name}')
        if re.search(r'(?i)(?:webview2loader\.dll|\.svelte$|\.m?js$|\.html$)', name):
            raise SystemExit(f'Unexpected web runtime payload: {name}')
with zipfile.ZipFile(args.directory / f'Nomo_{version}_x64.zip') as archive:
    names = set(archive.namelist())
    validate_paths(names)
    if not {'Nomo/nomo.exe', 'Nomo/vcruntime140.dll'} <= names:
        raise SystemExit('Invalid portable ZIP layout')
    if archive.testzip() is not None:
        raise SystemExit('Corrupt portable ZIP')
with tarfile.open(args.directory / 'Nomo_aarch64.app.tar.gz') as archive:
    validate_paths(archive.getnames())
    if 'Nomo.app/Contents/MacOS/nomo' not in archive.getnames():
        raise SystemExit('Missing macOS app executable')
lines = []
for name in sorted(expected):
    with (args.directory / name).open('rb') as stream:
        digest = hashlib.file_digest(stream, 'md5').hexdigest()
    lines.append(f'{digest}  {name}\n')
(args.directory / 'checksums.md5').write_text(''.join(lines), encoding='utf-8')
print(f'Validated {len(expected)} assets for {args.tag}')
