const got = require('got');
const toml = require('toml');
const fs = require('mz/fs');
const pall = require('p-all');
const minimist = require('minimist');
const semver = require('semver');

async function getLicenseFromAPI(repository) {
	const res = await got(`https://api.github.com/repos/${repository}/license`, {
		json: true,
		auth: process.env['GITHUB_KEY'],
		headers: {
			Accept: 'application/vnd.github.v3+json'
		}
	});

	return Buffer.from(res.body.content, 'base64').toString('utf8');
}

async function getFileFromRepository(repository, file) {
	const res = await got(`https://raw.githubusercontent.com/${repository}/master/${file}`, { auth: process.env['GITHUB_KEY'] });
	return res.body;
}

async function getLicenseFromRepository(repository) {
	try {
		return (await getFileFromRepository(repository, 'LICENSE-MIT'));
	} catch (err) {
		try {
			return (await getFileFromRepository(repository, 'LICENSE-APACHE'));
		} catch (err) {
			try {
				return (await getFileFromRepository(repository, 'LICENSE'));
			} catch (err) {
				return (await getFileFromRepository(repository, 'LICENSE.md'));
			}
		}
	}
}

async function getLicense(repository) {
	try {
		return await getLicenseFromRepository(repository);
	} catch (err) {
		return await getLicenseFromAPI(repository);
	}
}

async function getCrateInfo(name) {
	const res = await got(`https://crates.io/api/v1/crates/${name}`, { json: true });
	return res.body;
}

function comparePackages(a, b) {
	if (a.name === b.name) {
		return semver.compare(a.version, b.version);
	}

	return a.name < b.name ? -1 : 1;
}

async function main(argv) {
	if (argv._.length < 1) {
		throw new Error('Usage: node legal [--ossreadme] Cargo.lock');
	}

	const ossreadme = [];

	console.error('Checking OSS dependencies for MIT license...');

	const raw = await fs.readFile(argv._[0], 'utf8');
	const cargolock = toml.parse(raw);
	const tasks = cargolock.package
		.filter(pkg => pkg.name !== 'inno_updater')
		.map(pkg => async () => {
			const info = await getCrateInfo(pkg.name);
			const versionInfo = info.versions.filter(v => v.num === pkg.version)[0];

			if (argv['ossreadme']) {
				const repositoryUrl = info.crate.repository;
				const match = /github\.com\/([^/]+\/[^/]+)(\/|$)/.exec(repositoryUrl);

				if (!match) {
					console.error(`${pkg.name} does not live in github: ${repositoryUrl}`);
					process.exit(1);
				}

				const repository = match[1].replace(/\.git$/, '');
				const license = await getLicense(repository);

				ossreadme.push({
					name: repository,
					version: versionInfo.num,
					repositoryUrl,
					licenseDetail: license.trim().split(/\r?\n/g),
					isProd: true
				})
			}

			const isMIT = /MIT/.test(versionInfo.license);

			console.error(`${versionInfo.crate} ${versionInfo.num} ${versionInfo.license} ${isMIT ? '✔︎' : '✖︎'}`);
			return isMIT;
		});

	const areMIT = await pall(tasks, { concurrency: 10 });
	const allAreMIT = areMIT.reduce((r, v) => r && v, true);

	if (!allAreMIT) {
		throw new Error('Some dependencies are not MIT!');
	}

	if (argv['ossreadme']) {
		ossreadme.sort(comparePackages);
		console.log(JSON.stringify(ossreadme, null, '\t'));
	}

	return;
}

const opts = {
	boolean: 'ossreadme'
};

main(minimist(process.argv.slice(2), opts))
	.catch(err => { console.error(err.message); return 1; })
	.then(result => process.exit(result));