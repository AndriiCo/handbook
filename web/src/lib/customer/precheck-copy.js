// The /precheck demo's words in the three site languages (lib/i18n.ts
// LOCALES), picked by the shared site locale (lib/locale.svelte.ts).
// Sample names, issue lines, box labels, model names and the recording
// stamp come from the recorded verdicts and stay as recorded.

/**
 * @typedef {{
 *   title: string,
 *   heading: string,
 *   intro: string,
 *   samplesUnavailable: string,
 *   loadFailed: string,
 *   loadingSamples: string,
 *   samplesLabel: string,
 *   checking: string,
 *   runAgain: string,
 *   run: string,
 *   runUnavailable: string,
 *   tooManyRuns: string,
 *   recording: string,
 *   runFailed: string,
 *   resultLabel: string,
 *   pass: string,
 *   retake: string,
 *   checklistLabel: string,
 *   ready: string,
 *   unrecorded: string,
 *   footer: (models: string, recorded: string) => string
 * }} PrecheckCopy
 */

/** @type {Record<'en' | 'es' | 'fr', PrecheckCopy>} */
export const PRECHECK_COPY = {
	en: {
		title: 'Precheck',
		heading: 'Document precheck',
		intro:
			'Try the photo check on a synthetic sample. SAM boxes draw over the page; the verdict names its models and recording date.',
		samplesUnavailable: 'Samples unavailable.',
		loadFailed: 'Could not load the demo samples — try again.',
		loadingSamples: 'Loading samples',
		samplesLabel: 'Demo samples',
		checking: 'Checking…',
		runAgain: 'Run again',
		run: 'Run precheck',
		runUnavailable: 'Run unavailable.',
		tooManyRuns: 'Too many runs — wait a minute and try again.',
		recording: 'Demo verdicts are being recorded — check back after the next deploy.',
		runFailed: 'The precheck run failed — try again.',
		resultLabel: 'Precheck result',
		pass: 'Pass — good to submit',
		retake: 'Retake — fix the issues below',
		checklistLabel: 'Precheck checklist',
		ready: 'Ready to submit — no defects found',
		unrecorded: 'unrecorded',
		footer: (models, recorded) =>
			`models ${models} · recorded ${recorded} · replay of a live run on synthetic samples`
	},
	es: {
		title: 'Verificación previa',
		heading: 'Verificación previa del documento',
		intro:
			'Pruebe la revisión de fotos con una muestra sintética. Los recuadros de SAM se dibujan sobre la página; el veredicto indica sus modelos y la fecha de grabación.',
		samplesUnavailable: 'Muestras no disponibles.',
		loadFailed: 'No se pudieron cargar las muestras de demostración: inténtelo de nuevo.',
		loadingSamples: 'Cargando las muestras',
		samplesLabel: 'Muestras de demostración',
		checking: 'Comprobando…',
		runAgain: 'Ejecutar de nuevo',
		run: 'Ejecutar la verificación',
		runUnavailable: 'Ejecución no disponible.',
		tooManyRuns: 'Demasiadas ejecuciones: espere un minuto e inténtelo de nuevo.',
		recording:
			'Se están grabando los veredictos de demostración: vuelva a consultar después del próximo despliegue.',
		runFailed: 'La verificación previa falló: inténtelo de nuevo.',
		resultLabel: 'Resultado de la verificación previa',
		pass: 'Aprobado: listo para enviar',
		retake: 'Repetir la foto: corrija los problemas de abajo',
		checklistLabel: 'Lista de comprobación',
		ready: 'Listo para enviar: no se encontraron defectos',
		unrecorded: 'sin registrar',
		footer: (models, recorded) =>
			`modelos ${models} · grabado ${recorded} · repetición de una ejecución real sobre muestras sintéticas`
	},
	fr: {
		title: 'Vérification préalable',
		heading: 'Vérification préalable du document',
		intro:
			'Essayez la vérification photo sur un échantillon synthétique. Les cadres SAM se dessinent sur la page ; le verdict indique ses modèles et sa date d’enregistrement.',
		samplesUnavailable: 'Échantillons indisponibles.',
		loadFailed: 'Impossible de charger les échantillons de démonstration : réessayez.',
		loadingSamples: 'Chargement des échantillons',
		samplesLabel: 'Échantillons de démonstration',
		checking: 'Vérification…',
		runAgain: 'Relancer',
		run: 'Lancer la vérification',
		runUnavailable: 'Lancement indisponible.',
		tooManyRuns: 'Trop de lancements : patientez une minute puis réessayez.',
		recording:
			'Les verdicts de démonstration sont en cours d’enregistrement : revenez après le prochain déploiement.',
		runFailed: 'La vérification préalable a échoué : réessayez.',
		resultLabel: 'Résultat de la vérification préalable',
		pass: 'Conforme : prêt à envoyer',
		retake: 'À refaire : corrigez les problèmes ci-dessous',
		checklistLabel: 'Liste de contrôle',
		ready: 'Prêt à envoyer : aucun défaut détecté',
		unrecorded: 'non enregistrés',
		footer: (models, recorded) =>
			`modèles ${models} · enregistré ${recorded} · rediffusion d’une exécution réelle sur des échantillons synthétiques`
	}
};
