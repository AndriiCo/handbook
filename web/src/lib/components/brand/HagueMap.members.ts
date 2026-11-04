// Hague membership for the map, from staff-accepted HCCH facts only
// (topics `jurisdiction` / `legalization_chain`). The map never knows a
// member: a country is tinted only when an accepted statement names it
// as a party. Names come from the browser's CLDR region names; the
// table below is the ISO 3166-1 numeric → alpha-2 key for the
// precomputed paths in geo/world.json (reference data, not a fact).

const ISO_NUMERIC_TO_ALPHA2 = Object.fromEntries(
	(
		'242FJ 834TZ 732EH 124CA 840US 398KZ 860UZ 598PG 360ID 032AR 152CL 180CD 706SO 404KE ' +
		'729SD 148TD 332HT 214DO 643RU 044BS 238FK 578NO 304GL 260TF 626TL 710ZA 426LS 484MX ' +
		'858UY 076BR 068BO 604PE 170CO 591PA 188CR 558NI 340HN 222SV 320GT 084BZ 862VE 328GY ' +
		'740SR 250FR 218EC 630PR 388JM 192CU 716ZW 072BW 516NA 686SN 466ML 478MR 204BJ 562NE ' +
		'566NG 120CM 768TG 288GH 384CI 324GN 624GW 430LR 694SL 854BF 140CF 178CG 266GA 226GQ ' +
		'894ZM 454MW 508MZ 748SZ 024AO 108BI 376IL 422LB 450MG 275PS 270GM 788TN 012DZ 400JO ' +
		'784AE 634QA 414KW 368IQ 512OM 548VU 116KH 764TH 418LA 104MM 704VN 408KP 410KR 496MN ' +
		'356IN 050BD 064BT 524NP 586PK 004AF 762TJ 417KG 795TM 364IR 760SY 051AM 752SE 112BY ' +
		'804UA 616PL 040AT 348HU 498MD 642RO 440LT 428LV 233EE 276DE 100BG 300GR 792TR 008AL ' +
		'191HR 756CH 442LU 056BE 528NL 620PT 724ES 372IE 540NC 090SB 554NZ 036AU 144LK 156CN ' +
		'158TW 380IT 208DK 826GB 352IS 031AZ 268GE 608PH 458MY 096BN 705SI 246FI 703SK 203CZ ' +
		'232ER 392JP 600PY 887YE 682SA 010AQ 196CY 504MA 818EG 434LY 231ET 262DJ 800UG 646RW ' +
		'070BA 807MK 688RS 499ME 780TT 728SS'
	)
		.split(' ')
		.map((pair) => [pair.slice(0, 3), pair.slice(3)])
);

type Fact = { topic?: unknown; status?: unknown; statement?: unknown };

const MEMBERSHIP_TOPICS = new Set(['jurisdiction', 'legalization_chain']);
// A sentence about leaving or not joining never tints a country.
const NEGATION = /\b(not|no longer|non-member|denounc\w*|withdr\w*)\b/i;

function tokens(s: string): string[] {
	return s
		.normalize('NFKD')
		.replace(/[̀-ͯ]/g, '')
		.toLowerCase()
		.replace(/&/g, ' and ')
		.split(/[^a-z]+/)
		.filter(Boolean);
}

let names: { id: string; parts: string[]; joined: string }[] | null = null;

function countryNames() {
	if (names) return names;
	names = [];
	try {
		const styles = (['long', 'short'] as const).map(
			(style) => new Intl.DisplayNames(['en'], { type: 'region', style })
		);
		for (const [id, alpha2] of Object.entries(ISO_NUMERIC_TO_ALPHA2)) {
			const seen = new Set<string>();
			for (const dn of styles) {
				const name = dn.of(alpha2);
				if (!name || name === alpha2) continue;
				// "Congo - Kinshasa", "Myanmar (Burma)": the head names it.
				const parts = tokens(name.split(/\s+-\s+|\s*\(/)[0]);
				const joined = parts.join('');
				// Two-letter short forms ("US", "UK") read as ordinary words.
				if (joined.length < 4 || seen.has(joined)) continue;
				seen.add(joined);
				names.push({ id, parts, joined });
			}
		}
	} catch {
		// No Intl region names: an honest empty map, never a guess.
	}
	// Longest names claim their words first ("Papua New Guinea"
	// before "Guinea", "South Sudan" before "Sudan").
	names.sort((a, b) => b.parts.length - a.parts.length || b.joined.length - a.joined.length);
	return names;
}

/**
 * ISO numeric ids (world.json keys) of the countries that accepted
 * HCCH facts name as parties. Empty when no accepted fact names one.
 */
export function membersFromFacts(facts: unknown): string[] {
	const out = new Set<string>();
	const list = (Array.isArray(facts) ? facts : []) as Fact[];
	for (const f of list) {
		if (f?.status !== 'accepted' || !MEMBERSHIP_TOPICS.has(String(f?.topic ?? ''))) continue;
		for (const sentence of String(f?.statement ?? '').split(/(?<=[.;])\s+/)) {
			if (NEGATION.test(sentence)) continue;
			const words = tokens(sentence);
			const used = new Array(words.length).fill(false);
			for (const n of countryNames()) {
				for (let i = 0; i < words.length; i++) {
					// Exact word run ("united states"), or a run that joins
					// to the name ("viet nam" = "vietnam").
					let hit = 0;
					if (n.parts.every((p, k) => words[i + k] === p)) hit = n.parts.length;
					else {
						let acc = '';
						for (let k = i; k < Math.min(words.length, i + 3); k++) {
							acc += words[k];
							if (acc === n.joined) {
								hit = k - i + 1;
								break;
							}
						}
					}
					if (!hit) continue;
					if (used.slice(i, i + hit).some(Boolean)) continue;
					used.fill(true, i, i + hit);
					out.add(n.id);
				}
			}
		}
	}
	return [...out];
}

/** Lowercase alpha-2 of every country the world map draws (route pickers). */
export const WORLD_ALPHA2: string[] = Object.values(ISO_NUMERIC_TO_ALPHA2).map((a) =>
	a.toLowerCase()
);

/**
 * `membersFromFacts` as lowercase alpha-2 codes (the /route method
 * check): the same staff-accepted HCCH statements, the same honest
 * empty list when none names a party.
 */
export function memberAlpha2FromFacts(facts: unknown): string[] {
	return membersFromFacts(facts)
		.map((id) => ISO_NUMERIC_TO_ALPHA2[id]?.toLowerCase())
		.filter((a): a is string => Boolean(a));
}
