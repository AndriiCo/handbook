// The /uploads page's words in the three site languages (lib/i18n.ts
// LOCALES), picked by the shared site locale (lib/locale.svelte.ts).
// The file checks' own messages live with the checks (uploads.js).

/**
 * @typedef {{
 *   title: string,
 *   intro: string,
 *   choose: string,
 *   cannotUpload: string,
 *   uploadedHeading: string,
 *   uploaded: (n: number) => string,
 *   notFound: string,
 *   failed: (status: number) => string,
 *   unreachable: string,
 *   uploading: string,
 *   upload: string
 * }} UploadsCopy
 */

/** @type {Record<'en' | 'es' | 'fr', UploadsCopy>} */
export const UPLOADS_COPY = {
	en: {
		title: 'Uploads',
		intro:
			'Document photos and scans for your orders. Originals stay private to your account and staff — community-visible copies are masked first.',
		choose: 'Choose files (up to 10, 25 MB each)',
		cannotUpload: 'Cannot upload.',
		uploadedHeading: 'Uploaded.',
		uploaded: (n) => `${n} file${n === 1 ? '' : 's'} uploaded.`,
		notFound: 'Upload endpoint not found (404) — the server lane has not landed yet.',
		failed: (status) => `Upload failed: HTTP ${status}. Nothing was stored.`,
		unreachable: 'Upload service unreachable — is the API server running? Nothing was sent.',
		uploading: 'Uploading…',
		upload: 'Upload'
	},
	es: {
		title: 'Archivos',
		intro:
			'Fotos y escaneos de documentos para sus pedidos. Los originales solo los ven usted y el personal; las copias visibles para la comunidad se enmascaran antes.',
		choose: 'Elija archivos (hasta 10, de 25 MB cada uno)',
		cannotUpload: 'No se puede subir.',
		uploadedHeading: 'Subido.',
		uploaded: (n) => (n === 1 ? '1 archivo subido.' : `${n} archivos subidos.`),
		notFound:
			'No se encontró el servicio de subida (404): la parte del servidor todavía no está disponible.',
		failed: (status) => `La subida falló: HTTP ${status}. No se guardó nada.`,
		unreachable:
			'No se puede contactar con el servicio de subida: ¿está en marcha el servidor de la API? No se envió nada.',
		uploading: 'Subiendo…',
		upload: 'Subir'
	},
	fr: {
		title: 'Fichiers',
		intro:
			'Photos et numérisations de documents pour vos commandes. Les originaux restent visibles uniquement par vous et le personnel ; les copies visibles par la communauté sont d’abord masquées.',
		choose: 'Choisissez des fichiers (10 au maximum, 25 Mo chacun)',
		cannotUpload: 'Envoi impossible.',
		uploadedHeading: 'Envoyé.',
		uploaded: (n) => (n <= 1 ? `${n} fichier envoyé.` : `${n} fichiers envoyés.`),
		notFound:
			'Service d’envoi introuvable (404) : la partie serveur n’est pas encore en place.',
		failed: (status) => `L’envoi a échoué : HTTP ${status}. Rien n’a été enregistré.`,
		unreachable:
			'Service d’envoi injoignable : le serveur d’API est-il démarré ? Rien n’a été envoyé.',
		uploading: 'Envoi en cours…',
		upload: 'Envoyer'
	}
};
