"""Build the guest workspace and install every manifest-declared WASM module."""
import json
from pathlib import Path
import shutil
import subprocess
root=Path(__file__).resolve().parent
cargo=shutil.which('cargo') or str(Path.home()/'.cargo/bin/cargo')
subprocess.run([cargo,'build','--manifest-path',str(root/'Cargo.toml'),'--target','wasm32-unknown-unknown','--release','--locked','--offline'],check=True)
metadata=json.loads(subprocess.check_output([cargo,'metadata','--manifest-path',str(root/'Cargo.toml'),'--no-deps','--format-version','1','--locked','--offline']))
packages={Path(package['manifest_path']).parent.resolve():package for package in metadata['packages']}
artifacts=Path(metadata['target_directory'])/'wasm32-unknown-unknown/release'
for manifest_path in sorted(root.glob('*/manifest.json')):
    manifest=json.loads(manifest_path.read_text())
    package=packages[manifest_path.parent.resolve()]
    target=next(target['name'] for target in package['targets'] if 'cdylib' in target['crate_types'])
    module=manifest['module']
    if Path(module).name!=module or not module.endswith('.wasm'):
        raise ValueError(f'Expected a local .wasm module: {manifest_path}')
    shutil.copyfile(artifacts/f'{target}.wasm',manifest_path.parent/module)
fixture=root.parent/'crates/acheflow-server/tests/fixtures/domain.wasm'
fixture.parent.mkdir(parents=True,exist_ok=True)
shutil.copyfile(artifacts/'acheflow_extension_fixture.wasm',fixture)
