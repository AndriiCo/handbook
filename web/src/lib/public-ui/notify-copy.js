// The update-email pages' words (/notify/confirm, /notify/unsubscribe)
// in the three site languages (lib/i18n.ts LOCALES), picked by the shared
// site locale so the masthead switcher re-renders them in place.

/**
 * @typedef {{
 *   backToIndex: string,
 *   confirmTitle: string,
 *   confirmHeading: string,
 *   confirming: string,
 *   followingHeading: string,
 *   followingBody: string,
 *   expiredHeading: string,
 *   expiredBody: string,
 *   unsubscribeTitle: string,
 *   unsubscribeHeading: string,
 *   unsubscribedHeading: string,
 *   unsubscribedBody: string,
 *   invalidHeading: string,
 *   invalidBody: string,
 *   pressButton: string,
 *   working: string,
 *   unsubscribe: string,
 *   failed: string,
 *   pasteHint: string,
 *   tokenLabel: string,
 *   tokenPlaceholder: string
 * }} NotifyCopy
 */

/** @type {Record<'en' | 'es' | 'fr', NotifyCopy>} */
export const NOTIFY_COPY = {
	en: {
		backToIndex: 'Back to the authority index',
		confirmTitle: 'Confirm updates — Apostille Handbook',
		confirmHeading: 'Confirm email updates',
		confirming: 'Confirming subscription',
		followingHeading: 'You are following this office.',
		followingBody:
			'You will get one email when staff accept a changed fact for it — at most one per hour. Every email carries a one-click unsubscribe link.',
		expiredHeading: 'This confirm link is invalid or expired.',
		expiredBody:
			'Links die 24 hours after they are sent and work only once. Follow the office again from its dossier page for a fresh link.',
		unsubscribeTitle: 'Unsubscribe — Apostille Handbook',
		unsubscribeHeading: 'Unsubscribe from updates',
		unsubscribedHeading: 'Unsubscribed.',
		unsubscribedBody:
			'No more update emails for this office. The address stays on file only as a hash so a later resubscribe is instant — write to us if you want it fully erased.',
		invalidHeading: 'This unsubscribe link is invalid or expired.',
		invalidBody:
			'Open the one-click link from a recent update email, or follow the office again from its dossier page for a fresh link.',
		pressButton: 'Press the button below to stop update emails for this subscription.',
		working: 'Working…',
		unsubscribe: 'Unsubscribe',
		failed: 'Something went wrong — please try again.',
		pasteHint:
			'Paste the token from your email footer, or open the one-click link from any update email.',
		tokenLabel: 'Unsubscribe token',
		tokenPlaceholder: '64-character token'
	},
	es: {
		backToIndex: 'Volver al índice de autoridades',
		confirmTitle: 'Confirmar avisos — Apostille Handbook',
		confirmHeading: 'Confirmar los avisos por correo electrónico',
		confirming: 'Confirmando la suscripción',
		followingHeading: 'Ahora sigue esta oficina.',
		followingBody:
			'Recibirá un correo cuando el personal acepte un dato modificado de esta oficina, como máximo uno por hora. Cada correo incluye un enlace para darse de baja con un clic.',
		expiredHeading: 'Este enlace de confirmación no es válido o ha caducado.',
		expiredBody:
			'Los enlaces caducan 24 horas después de enviarse y solo funcionan una vez. Vuelva a seguir la oficina desde su expediente para recibir un enlace nuevo.',
		unsubscribeTitle: 'Darse de baja — Apostille Handbook',
		unsubscribeHeading: 'Darse de baja de los avisos',
		unsubscribedHeading: 'Se ha dado de baja.',
		unsubscribedBody:
			'No recibirá más avisos de esta oficina. La dirección solo se conserva como hash para que una nueva suscripción sea inmediata; escríbanos si desea que se borre por completo.',
		invalidHeading: 'Este enlace para darse de baja no es válido o ha caducado.',
		invalidBody:
			'Abra el enlace de un clic de un aviso reciente, o vuelva a seguir la oficina desde su expediente para recibir un enlace nuevo.',
		pressButton: 'Pulse el botón para dejar de recibir avisos de esta suscripción.',
		working: 'Procesando…',
		unsubscribe: 'Darse de baja',
		failed: 'Algo salió mal: inténtelo de nuevo.',
		pasteHint:
			'Pegue el código que aparece al pie de su correo, o abra el enlace de un clic de cualquier aviso.',
		tokenLabel: 'Código para darse de baja',
		tokenPlaceholder: 'Código de 64 caracteres'
	},
	fr: {
		backToIndex: 'Retour au répertoire des autorités',
		confirmTitle: 'Confirmer les alertes — Apostille Handbook',
		confirmHeading: 'Confirmer les alertes par e-mail',
		confirming: 'Confirmation de l’abonnement',
		followingHeading: 'Vous suivez désormais ce bureau.',
		followingBody:
			'Vous recevrez un e-mail lorsque le personnel acceptera un fait modifié pour ce bureau, au plus un par heure. Chaque e-mail contient un lien de désabonnement en un clic.',
		expiredHeading: 'Ce lien de confirmation est invalide ou a expiré.',
		expiredBody:
			'Les liens expirent 24 heures après leur envoi et ne fonctionnent qu’une fois. Suivez de nouveau le bureau depuis son dossier pour recevoir un nouveau lien.',
		unsubscribeTitle: 'Se désabonner — Apostille Handbook',
		unsubscribeHeading: 'Se désabonner des alertes',
		unsubscribedHeading: 'Désabonnement effectué.',
		unsubscribedBody:
			'Vous ne recevrez plus d’alertes pour ce bureau. L’adresse n’est conservée que sous forme d’empreinte (hash) afin qu’un réabonnement soit immédiat : écrivez-nous si vous souhaitez qu’elle soit entièrement effacée.',
		invalidHeading: 'Ce lien de désabonnement est invalide ou a expiré.',
		invalidBody:
			'Ouvrez le lien en un clic d’une alerte récente, ou suivez de nouveau le bureau depuis son dossier pour recevoir un nouveau lien.',
		pressButton: 'Appuyez sur le bouton pour ne plus recevoir d’alertes pour cet abonnement.',
		working: 'Traitement…',
		unsubscribe: 'Se désabonner',
		failed: 'Une erreur s’est produite : veuillez réessayer.',
		pasteHint:
			'Collez le code figurant en bas de votre e-mail, ou ouvrez le lien en un clic de n’importe quelle alerte.',
		tokenLabel: 'Code de désabonnement',
		tokenPlaceholder: 'Code de 64 caractères'
	}
};
