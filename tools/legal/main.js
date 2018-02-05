const got = require('got');
const toml = require('toml');
const fs = require('mz/fs');
const pall = require('p-all');

async function getCrateInfo(name, version) {
	const res = await got(`https://crates.io/api/v1/crates/${name}/${version}`, { json: true });
	return res.body;
}

async function main(argv) {
	if (argv.length < 3) {
		throw new Error('Usage: node legal [Cargo.lock]');
	}

	console.log('Checking OSS dependencies for MIT license...');

	const raw = await fs.readFile(argv[2], 'utf8');
	const cargolock = toml.parse(raw);
	const tasks = cargolock.package
		.filter(pkg => pkg.name !== 'inno_updater')
		.map(pkg => async () => {
			const info = await getCrateInfo(pkg.name, pkg.version);
			const isMIT = /MIT/.test(info.version.license);

			console.log(`${info.version.crate} ${info.version.num} ${info.version.license} ${isMIT ? '✔︎' : '✖︎'}`);

			return isMIT;
		});

	const areMIT = await pall(tasks, { concurrency: 10 });
	const ok = areMIT.reduce((r, v) => r && v, true);

	if (!ok) {
		console.error('Some dependencies are not MIT!');
	}

	return ok ? 0 : 1;
}

main(process.argv)
	.catch(err => { console.error(err.message); return 1; })
	.then(result => process.exit(result));