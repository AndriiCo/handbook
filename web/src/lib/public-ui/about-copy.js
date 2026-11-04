// The "How we verify" page's words (/about) in the three site languages
// (lib/i18n.ts LOCALES): the eval line and the three checks that used to
// sit on the home page, for readers who want the method behind the rules.

import { num } from './home-copy.js';

/** @typedef {'en' | 'es' | 'fr'} Locale */

/**
 * @typedef {{
 *   headTitle: string,
 *   metaDescription: string,
 *   eyebrow: string,
 *   title: string,
 *   intro: string,
 *   evalLine: (pct: number) => string,
 *   howTitle: string,
 *   steps: { model: string, title: string, text: string }[],
 *   citationsTitle: string,
 *   citations: string,
 *   backHome: string
 * }} AboutCopy
 */

/** @type {Record<Locale, AboutCopy>} */
export const ABOUT_COPY = {
	en: {
		headTitle: 'How we verify — Apostille Handbook',
		metaDescription:
			'Every requirement in the Apostille Handbook is quoted from an official government page, checked word for word, and approved by a person.',
		eyebrow: 'About',
		title: 'How we verify every rule',
		intro:
			'Every requirement comes from the office’s own pages. Nothing is written from memory, and nothing reaches a guide until a person has approved it.',
		evalLine: (pct) => `Eval: ${pct}% of Muse Spark drafts quote the source verbatim`,
		howTitle: 'Three checks before anything is published',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark drafts from official pages',
				text: 'Every fact starts as a quote copied from a watched government page — never from memory.'
			},
			{
				model: 'Quote check',
				title: 'The quote is checked byte-exact',
				text: 'A refused or missing quote goes back to the model once; unverifiable drafts never publish.'
			},
			{
				model: 'Human review',
				title: 'Staff approve, the dossier updates live',
				text: 'Only accepted facts reach the public dossier, seconds after approval.'
			}
		],
		citationsTitle: 'See the source yourself',
		citations:
			'On each guide, citations are off by default: turn them on with Alt+C or the Citations button to see the quote, source and date.',
		backHome: 'Back to the start'
	},
	es: {
		headTitle: 'Cómo verificamos — Apostille Handbook',
		metaDescription:
			'Cada requisito del Apostille Handbook se cita de una página oficial del gobierno, se comprueba palabra por palabra y lo aprueba una persona.',
		eyebrow: 'Acerca de',
		title: 'Cómo verificamos cada regla',
		intro:
			'Cada requisito procede de las páginas de la propia oficina. Nada se escribe de memoria y nada llega a una guía sin que una persona lo apruebe.',
		evalLine: (pct) =>
			`Evaluación: el ${num(pct, 'es')} % de los borradores de Muse Spark cita la fuente textualmente`,
		howTitle: 'Tres comprobaciones antes de publicar nada',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark redacta a partir de páginas oficiales',
				text: 'Cada dato parte de una cita copiada de una página oficial vigilada, nunca de memoria.'
			},
			{
				model: 'Comprobación de la cita',
				title: 'La cita se comprueba byte a byte',
				text: 'Una cita rechazada o ausente vuelve una vez al modelo; los borradores no verificables nunca se publican.'
			},
			{
				model: 'Revisión humana',
				title: 'El personal aprueba y el expediente se actualiza al instante',
				text: 'Solo los datos aceptados llegan al expediente público, segundos después de su aprobación.'
			}
		],
		citationsTitle: 'Vea la fuente usted mismo',
		citations:
			'En cada guía, las citas están ocultas por defecto: actívelas con Alt+C o con el botón Citas para ver la cita, la fuente y la fecha.',
		backHome: 'Volver al inicio'
	},
	fr: {
		headTitle: 'Notre méthode de vérification — Apostille Handbook',
		metaDescription:
			'Chaque exigence de l’Apostille Handbook est citée d’une page gouvernementale officielle, vérifiée mot pour mot et approuvée par une personne.',
		eyebrow: 'À propos',
		title: 'Comment nous vérifions chaque règle',
		intro:
			'Chaque exigence provient des pages officielles du bureau concerné. Rien n’est écrit de mémoire, et rien n’arrive dans un guide sans l’accord d’une personne.',
		evalLine: (pct) =>
			`Évaluation : ${num(pct, 'fr')} % des brouillons de Muse Spark citent la source mot pour mot`,
		howTitle: 'Trois vérifications avant toute publication',
		steps: [
			{
				model: 'Muse Spark',
				title: 'Spark rédige à partir des pages officielles',
				text: 'Chaque fait part d’une citation copiée d’une page gouvernementale surveillée, jamais de mémoire.'
			},
			{
				model: 'Vérification de la citation',
				title: 'La citation est vérifiée à l’octet près',
				text: 'Une citation refusée ou manquante repart une fois vers le modèle ; un brouillon invérifiable n’est jamais publié.'
			},
			{
				model: 'Relecture humaine',
				title: 'Le personnel approuve, le dossier se met à jour en direct',
				text: 'Seuls les faits acceptés arrivent dans le dossier public, quelques secondes après approbation.'
			}
		],
		citationsTitle: 'Voyez la source par vous-même',
		citations:
			'Dans chaque guide, les citations sont masquées par défaut : affichez-les avec Alt+C ou le bouton Citations pour voir la citation, la source et la date.',
		backHome: 'Retour à l’accueil'
	}
};
