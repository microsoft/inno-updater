const got = require('got');
const toml = require('toml');
const fs = require('mz/fs');
const pall = require('p-all');
const minimist = require('minimist');

async function getCrateInfo(name) {
	const res = await got(`https://crates.io/api/v1/crates/${name}`, { json: true });
	return res.body;
}

async function main(argv) {
	if (argv._.length < 1) {
		throw new Error('Usage: node legal Cargo.lock');
	}

	console.error('Checking OSS dependencies for MIT license...');

	const raw = await fs.readFile(argv._[0], 'utf8');
	const cargolock = toml.parse(raw);
	const tasks = cargolock.package
		.filter(pkg => pkg.name !== 'inno_updater')
		.map(pkg => async () => {
			const info = await getCrateInfo(pkg.name);
			const versionInfo = info.versions.filter(v => v.num === pkg.version)[0];
			const isMIT = /MIT/.test(versionInfo.license);

			console.error(`${versionInfo.crate} ${versionInfo.num} ${versionInfo.license} ${isMIT ? '✔︎' : '✖︎'}`);
			return isMIT;
		});

	const areMIT = await pall(tasks, { concurrency: 10 });
	const allAreMIT = areMIT.reduce((r, v) => r && v, true);

	if (!allAreMIT) {
		throw new Error('Some dependencies are not MIT!');
	}

	return;
}

main(minimist(process.argv.slice(2)))
	.catch(err => { console.error(err.message); return 1; })
	.then(result => process.exit(result));