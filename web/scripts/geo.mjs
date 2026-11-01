// geo.mjs — precompute map SVG paths at build time so no d3 ships to
// the browser. Reads world-atlas / us-atlas TopoJSON + d3-geo, writes
// src/lib/geo/*.json (path strings + precomputed pins/arcs).
//
// Usage: node scripts/geo.mjs
import { writeFileSync, mkdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { feature } from 'topojson-client';
import { geoNaturalEarth1, geoMercator, geoPath } from 'd3-geo';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src', 'lib', 'geo');
mkdirSync(outDir, { recursive: true });

const atlas = (pkg, file) =>
	JSON.parse(readFileSync(join(root, 'node_modules', pkg, file), 'utf8'));

// World (110m, equirectangular-family NaturalEarth, 960x480).
const W = 960;
const H = 480;
const worldTopo = atlas('world-atlas', 'countries-110m.json');
const worldProj = geoNaturalEarth1().fitSize([W, H], { type: 'Sphere' });
const worldPath = geoPath(worldProj);
const countries = feature(worldTopo, worldTopo.objects.countries).features.map((f) => ({
	id: String(f.id).padStart(3, '0'),
	d: worldPath(f) ?? ''
}));

// Pins + arcs, projected once here so the browser only renders strings.
// Seattle origin; featured authorities as labelled pins.
const pins = [
	{ slug: 'vietnam', label: 'Viet Nam', lon: 105.85, lat: 21.03 },
	{ slug: 'usa-wa-sos', label: 'Washington', lon: -122.9, lat: 47.04 },
	{ slug: 'hcch', label: 'The Hague', lon: 4.3, lat: 52.08 }
];
const seattle = worldProj([-122.3328, 47.6062]);
const pinPts = pins.map((p) => {
	const [x, y] = worldProj([p.lon, p.lat]);
	return { ...p, x: Math.round(x * 10) / 10, y: Math.round(y * 10) / 10 };
});
const arcs = pinPts
	.filter((p) => p.slug !== 'usa-wa-sos')
	.map((p) => {
		const [x0, y0] = seattle;
		const mx = (x0 + p.x) / 2;
		const my = Math.min(y0, p.y) - 60;
		return {
			slug: p.slug,
			d: `M${x0.toFixed(1)},${y0.toFixed(1)} Q${mx.toFixed(1)},${my.toFixed(1)} ${p.x},${p.y}`
		};
	});

writeFileSync(
	join(outDir, 'world.json'),
	JSON.stringify({ width: W, height: H, countries, pins: pinPts, arcs })
);

// Silhouettes: Vietnam (704) + Netherlands (528) from world, Washington
// state (FIPS 53) from us-atlas, each fitted to its own 200x140 box.
function silhouette(features, match, size = [200, 140]) {
	const f = features.find(match);
	const proj = geoMercator().fitSize(size, f);
	const d = geoPath(proj)(f) ?? '';
	return { d, width: size[0], height: size[1] };
}
const worldFeatures = feature(worldTopo, worldTopo.objects.countries).features;
const statesTopo = atlas('us-atlas', 'states-10m.json');
const stateFeatures = feature(statesTopo, statesTopo.objects.states).features;
const silhouettes = {
	vietnam: silhouette(worldFeatures, (f) => String(f.id).padStart(3, '0') === '704'),
	netherlands: silhouette(worldFeatures, (f) => String(f.id).padStart(3, '0') === '528'),
	washington: silhouette(stateFeatures, (f) => String(f.id).padStart(2, '0') === '53')
};
writeFileSync(join(outDir, 'silhouettes.json'), JSON.stringify(silhouettes));

const bytes = countries.reduce((n, c) => n + c.d.length, 0);
console.log(`geo: ${countries.length} countries (${bytes} path chars), 3 pins, 2 arcs, 3 silhouettes`);
