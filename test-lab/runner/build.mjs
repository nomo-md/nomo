import { cp, mkdir } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { lab, repo, walk, hashFile, hashJson, exists, json, saveJson, run, stamp } from './common.mjs';

export async function sourceFingerprint() {
  const files = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'test-lab/runner/cargo.ps1'].map(p => join(repo,p));
  for (const root of ['src-gpui/src','src-gpui/resources','src-gpui/assets','vendor/latex-rust/src']) files.push(...await walk(join(repo,root)));
  files.push(...['src-gpui/Cargo.toml','src-gpui/build.rs','vendor/latex-rust/Cargo.toml'].map(p=>join(repo,p)));
  return hashJson(await Promise.all(files.sort().map(async file=>[relative(repo,file).replaceAll('\\','/'),await hashFile(file)])));
}
export async function buildApp(configuration='release') {
  const fingerprint=await sourceFingerprint();
  const directory=join(lab,'.cache',`build-${stamp()}`);await mkdir(directory,{recursive:true});
  const log=join(directory,'build.log');console.log(`构建 GPUI automation ${configuration}：${log}`);
  await run('pwsh',['-NoProfile','-File',join(lab,'runner/cargo.ps1'),'-Configuration',configuration],{log});
  if (fingerprint!==await sourceFingerprint()) throw new Error('构建期间源码变化，请重新构建');
  const binary=join(repo,'target',configuration==='release'?'release':'debug','nomo.exe');
  const sha256=await hashFile(binary),id=`native-${configuration}-${sha256.slice(0,12)}`;
  const target=join(lab,'apps',id);await mkdir(target,{recursive:true});
  await cp(binary,join(target,'nomo.exe'));
  const manifest={protocol:3,backend:'gpui',id,configuration,sourceFingerprint:fingerprint,sha256,
    binary:join(target,'nomo.exe'),builtAt:new Date().toISOString(),features:['automation'],gpui:'0.3.10',
    commit:await run('git',['rev-parse','HEAD']),dirty:Boolean(await run('git',['status','--porcelain'])),log};
  await saveJson(join(target,'manifest.json'),manifest);await saveJson(join(lab,'apps',`latest-${configuration}.json`),manifest);
  return manifest;
}
export async function ensureBuild(configuration,{noBuild=false,build=false}={}) {
  const latest=join(lab,'apps',`latest-${configuration}.json`);
  const manifest=await exists(latest)?await json(latest):null;
  if(build||!manifest||manifest.protocol!==3||manifest.sourceFingerprint!==await sourceFingerprint()) {
    if(noBuild) throw new Error('没有与当前源码一致的原生测试构建；请先运行 build');
    return buildApp(configuration);
  }
  if(await hashFile(manifest.binary)!==manifest.sha256) throw new Error('测试二进制校验失败');
  return manifest;
}
