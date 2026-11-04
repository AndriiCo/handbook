// The /shipping page's words in the three site languages (lib/i18n.ts
// LOCALES), picked by the shared site locale (lib/locale.svelte.ts).
// Authority names, order ids, tracking numbers and carrier service
// names are data and stay as sent; the status badge and live pill
// (StatusBadge, LivePill) carry their own words.

import { ENUM_LABELS } from './orders-copy.js';

/**
 * @typedef {{
 *   title: string,
 *   eyebrow: string,
 *   signInHeading: string,
 *   signInBefore: string,
 *   signIn: string,
 *   signInAfter: string,
 *   errorHeading: string,
 *   tryLater: string,
 *   loading: string,
 *   empty: string,
 *   emptyBody: string,
 *   startOrder: string,
 *   summary: (shipments: number, orders: number) => string,
 *   kinds: { label_fedex: string, label_usps: string, mail_kit: string, address_label: string },
 *   roles: { kit_to_customer: string, outbound: string, return: string },
 *   copyTracking: (role: string) => string,
 *   noTracking: string,
 *   trackWith: (carrier: string) => string,
 *   labelPdf: string,
 *   copied: string,
 *   copyBlocked: string
 * }} ShippingCopy
 */

/** @type {Record<'en' | 'es' | 'fr', ShippingCopy>} */
export const SHIPPING_COPY = {
	en: {
		title: 'Shipping',
		eyebrow: 'Your account',
		signInHeading: 'Sign in to see your labels.',
		signInBefore: 'Labels and tracking live in your account.',
		signIn: 'Sign in',
		signInAfter: 'to continue.',
		errorHeading: 'Shipping is unavailable right now.',
		tryLater: 'Try again in a bit.',
		loading: 'Loading labels',
		empty: 'No labels yet.',
		emptyBody: 'Labels you buy with an order appear here with live tracking.',
		startOrder: 'Start an order',
		summary: (shipments, orders) =>
			`${shipments} ${shipments === 1 ? 'shipment' : 'shipments'} across ${orders} ${orders === 1 ? 'order' : 'orders'}.`,
		kinds: ENUM_LABELS.en.kind,
		roles: ENUM_LABELS.en.role,
		copyTracking: (role) => `Copy tracking number for ${role}`,
		noTracking: 'No tracking number on this label.',
		trackWith: (carrier) => `Track with ${carrier}`,
		labelPdf: 'Label PDF',
		copied: 'Tracking number copied.',
		copyBlocked: 'Copy is blocked in this browser; select the number instead.'
	},
	es: {
		title: 'Envíos',
		eyebrow: 'Su cuenta',
		signInHeading: 'Inicie sesión para ver sus etiquetas de envío.',
		signInBefore: 'Las etiquetas de envío y el seguimiento están en su cuenta.',
		signIn: 'Inicie sesión',
		signInAfter: 'para continuar.',
		errorHeading: 'Los envíos no están disponibles en este momento.',
		tryLater: 'Inténtelo de nuevo dentro de un rato.',
		loading: 'Cargando las etiquetas de envío',
		empty: 'Todavía no hay etiquetas de envío.',
		emptyBody:
			'Las etiquetas de envío que compre con un pedido aparecerán aquí con seguimiento en directo.',
		startOrder: 'Hacer un pedido',
		summary: (shipments, orders) =>
			`${shipments} ${shipments === 1 ? 'envío' : 'envíos'} en ${orders} ${orders === 1 ? 'pedido' : 'pedidos'}.`,
		kinds: ENUM_LABELS.es.kind,
		roles: ENUM_LABELS.es.role,
		copyTracking: (role) => `Copiar el número de seguimiento: ${role}`,
		noTracking: 'Esta etiqueta de envío no tiene número de seguimiento.',
		trackWith: (carrier) => `Seguir con ${carrier}`,
		labelPdf: 'Etiqueta en PDF',
		copied: 'Número de seguimiento copiado.',
		copyBlocked: 'Este navegador bloquea la copia; seleccione el número.'
	},
	fr: {
		title: 'Expéditions',
		eyebrow: 'Votre compte',
		signInHeading: 'Connectez-vous pour voir vos étiquettes d’expédition.',
		signInBefore: 'Les étiquettes d’expédition et le suivi se trouvent dans votre compte.',
		signIn: 'Connectez-vous',
		signInAfter: 'pour continuer.',
		errorHeading: 'Les expéditions sont indisponibles pour le moment.',
		tryLater: 'Réessayez un peu plus tard.',
		loading: 'Chargement des étiquettes d’expédition',
		empty: 'Aucune étiquette d’expédition pour l’instant.',
		emptyBody:
			'Les étiquettes d’expédition achetées avec une commande apparaissent ici, avec un suivi en direct.',
		startOrder: 'Passer une commande',
		summary: (shipments, orders) =>
			`${shipments} ${shipments <= 1 ? 'envoi' : 'envois'} sur ${orders} ${orders <= 1 ? 'commande' : 'commandes'}.`,
		kinds: ENUM_LABELS.fr.kind,
		roles: ENUM_LABELS.fr.role,
		copyTracking: (role) => `Copier le numéro de suivi : ${role}`,
		noTracking: 'Cette étiquette n’a pas de numéro de suivi.',
		trackWith: (carrier) => `Suivre chez ${carrier}`,
		labelPdf: 'Étiquette PDF',
		copied: 'Numéro de suivi copié.',
		copyBlocked: 'La copie est bloquée dans ce navigateur ; sélectionnez plutôt le numéro.'
	}
};
