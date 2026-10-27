// Form-resemblance framework: country list + autocomplete search.
//
// Static ISO 3166-1 list (alpha-2 code + English short name).
// EN-only per the slice rule; localization follows decisions 17-18.
// Pure functions, no fetching: the list compiles into the release.

/**
 * @typedef {{ code: string, name: string }} Country
 */

/** ISO 3166-1 alpha-2 code + English short name, alphabetical by name. */
export const COUNTRIES = [
	['AF', 'Afghanistan'],
	['AL', 'Albania'],
	['DZ', 'Algeria'],
	['AD', 'Andorra'],
	['AO', 'Angola'],
	['AG', 'Antigua and Barbuda'],
	['AR', 'Argentina'],
	['AM', 'Armenia'],
	['AU', 'Australia'],
	['AT', 'Austria'],
	['AZ', 'Azerbaijan'],
	['BS', 'Bahamas'],
	['BH', 'Bahrain'],
	['BD', 'Bangladesh'],
	['BB', 'Barbados'],
	['BY', 'Belarus'],
	['BE', 'Belgium'],
	['BZ', 'Belize'],
	['BJ', 'Benin'],
	['BT', 'Bhutan'],
	['BO', 'Bolivia'],
	['BA', 'Bosnia and Herzegovina'],
	['BW', 'Botswana'],
	['BR', 'Brazil'],
	['BN', 'Brunei'],
	['BG', 'Bulgaria'],
	['BF', 'Burkina Faso'],
	['BI', 'Burundi'],
	['CV', 'Cabo Verde'],
	['KH', 'Cambodia'],
	['CM', 'Cameroon'],
	['CA', 'Canada'],
	['CF', 'Central African Republic'],
	['TD', 'Chad'],
	['CL', 'Chile'],
	['CN', 'China'],
	['CO', 'Colombia'],
	['KM', 'Comoros'],
	['CG', 'Congo'],
	['CD', 'Congo, Democratic Republic of the'],
	['CR', 'Costa Rica'],
	['CI', "Côte d'Ivoire"],
	['HR', 'Croatia'],
	['CU', 'Cuba'],
	['CY', 'Cyprus'],
	['CZ', 'Czechia'],
	['DK', 'Denmark'],
	['DJ', 'Djibouti'],
	['DM', 'Dominica'],
	['DO', 'Dominican Republic'],
	['EC', 'Ecuador'],
	['EG', 'Egypt'],
	['SV', 'El Salvador'],
	['GQ', 'Equatorial Guinea'],
	['ER', 'Eritrea'],
	['EE', 'Estonia'],
	['SZ', 'Eswatini'],
	['ET', 'Ethiopia'],
	['FJ', 'Fiji'],
	['FI', 'Finland'],
	['FR', 'France'],
	['GA', 'Gabon'],
	['GM', 'Gambia'],
	['GE', 'Georgia'],
	['DE', 'Germany'],
	['GH', 'Ghana'],
	['GR', 'Greece'],
	['GD', 'Grenada'],
	['GT', 'Guatemala'],
	['GN', 'Guinea'],
	['GW', 'Guinea-Bissau'],
	['GY', 'Guyana'],
	['HT', 'Haiti'],
	['HN', 'Honduras'],
	['HU', 'Hungary'],
	['IS', 'Iceland'],
	['IN', 'India'],
	['ID', 'Indonesia'],
	['IR', 'Iran'],
	['IQ', 'Iraq'],
	['IE', 'Ireland'],
	['IL', 'Israel'],
	['IT', 'Italy'],
	['JM', 'Jamaica'],
	['JP', 'Japan'],
	['JO', 'Jordan'],
	['KZ', 'Kazakhstan'],
	['KE', 'Kenya'],
	['KI', 'Kiribati'],
	['KP', 'Korea, North'],
	['KR', 'Korea, South'],
	['KW', 'Kuwait'],
	['KG', 'Kyrgyzstan'],
	['LA', 'Laos'],
	['LV', 'Latvia'],
	['LB', 'Lebanon'],
	['LS', 'Lesotho'],
	['LR', 'Liberia'],
	['LY', 'Libya'],
	['LI', 'Liechtenstein'],
	['LT', 'Lithuania'],
	['LU', 'Luxembourg'],
	['MG', 'Madagascar'],
	['MW', 'Malawi'],
	['MY', 'Malaysia'],
	['MV', 'Maldives'],
	['ML', 'Mali'],
	['MT', 'Malta'],
	['MH', 'Marshall Islands'],
	['MR', 'Mauritania'],
	['MU', 'Mauritius'],
	['MX', 'Mexico'],
	['FM', 'Micronesia'],
	['MD', 'Moldova'],
	['MC', 'Monaco'],
	['MN', 'Mongolia'],
	['ME', 'Montenegro'],
	['MA', 'Morocco'],
	['MZ', 'Mozambique'],
	['MM', 'Myanmar'],
	['NA', 'Namibia'],
	['NR', 'Nauru'],
	['NP', 'Nepal'],
	['NL', 'Netherlands'],
	['NZ', 'New Zealand'],
	['NI', 'Nicaragua'],
	['NE', 'Niger'],
	['NG', 'Nigeria'],
	['MK', 'North Macedonia'],
	['NO', 'Norway'],
	['OM', 'Oman'],
	['PK', 'Pakistan'],
	['PW', 'Palau'],
	['PA', 'Panama'],
	['PG', 'Papua New Guinea'],
	['PY', 'Paraguay'],
	['PE', 'Peru'],
	['PH', 'Philippines'],
	['PL', 'Poland'],
	['PT', 'Portugal'],
	['QA', 'Qatar'],
	['RO', 'Romania'],
	['RU', 'Russia'],
	['RW', 'Rwanda'],
	['KN', 'Saint Kitts and Nevis'],
	['LC', 'Saint Lucia'],
	['VC', 'Saint Vincent and the Grenadines'],
	['WS', 'Samoa'],
	['SM', 'San Marino'],
	['ST', 'Sao Tome and Principe'],
	['SA', 'Saudi Arabia'],
	['SN', 'Senegal'],
	['RS', 'Serbia'],
	['SC', 'Seychelles'],
	['SL', 'Sierra Leone'],
	['SG', 'Singapore'],
	['SK', 'Slovakia'],
	['SI', 'Slovenia'],
	['SB', 'Solomon Islands'],
	['SO', 'Somalia'],
	['ZA', 'South Africa'],
	['SS', 'South Sudan'],
	['ES', 'Spain'],
	['LK', 'Sri Lanka'],
	['SD', 'Sudan'],
	['SR', 'Suriname'],
	['SE', 'Sweden'],
	['CH', 'Switzerland'],
	['SY', 'Syria'],
	['TW', 'Taiwan'],
	['TJ', 'Tajikistan'],
	['TZ', 'Tanzania'],
	['TH', 'Thailand'],
	['TL', 'Timor-Leste'],
	['TG', 'Togo'],
	['TO', 'Tonga'],
	['TT', 'Trinidad and Tobago'],
	['TN', 'Tunisia'],
	['TR', 'Türkiye'],
	['TM', 'Turkmenistan'],
	['TV', 'Tuvalu'],
	['UG', 'Uganda'],
	['UA', 'Ukraine'],
	['AE', 'United Arab Emirates'],
	['GB', 'United Kingdom'],
	['US', 'United States'],
	['UY', 'Uruguay'],
	['UZ', 'Uzbekistan'],
	['VU', 'Vanuatu'],
	['VA', 'Vatican City'],
	['VE', 'Venezuela'],
	['VN', 'Viet Nam'],
	['YE', 'Yemen'],
	['ZM', 'Zambia'],
	['ZW', 'Zimbabwe']
].map(
	([code, name]) =>
		/** @type {Country} */ ({ code, name })
);

/**
 * Case-insensitive, diacritic-sensitive substring search over country
 * names. Prefix matches rank first (in list order), then other
 * substring matches. Empty/blank query returns the first `limit`
 * countries. Matching is a convenience for the combobox — free text
 * stays allowed and validation decides.
 * @param {string} query
 * @param {number} [limit]
 * @returns {Country[]}
 */
export function searchCountries(query, limit = 10) {
	const q = query.trim().toLowerCase();
	if (q === '') return COUNTRIES.slice(0, limit);
	const prefix = [];
	const rest = [];
	for (const country of COUNTRIES) {
		const name = country.name.toLowerCase();
		if (name.startsWith(q)) prefix.push(country);
		else if (name.includes(q)) rest.push(country);
	}
	return [...prefix, ...rest].slice(0, limit);
}

/**
 * Exact name match (case-insensitive, trimmed).
 * @param {string} value
 * @returns {boolean}
 */
export function isCountryName(value) {
	const v = value.trim().toLowerCase();
	return COUNTRIES.some((c) => c.name.toLowerCase() === v);
}

/**
 * English short name for an alpha-2 code (case-insensitive), or
 * null when unknown.
 * @param {string} code
 * @returns {string | null}
 */
export function countryNameForCode(code) {
	const c = COUNTRIES.find((c) => c.code === code.trim().toUpperCase());
	return c ? c.name : null;
}
