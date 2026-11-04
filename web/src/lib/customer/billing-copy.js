// The /billing page's words in the three site languages (lib/i18n.ts
// LOCALES). The page title itself is the masthead's `billingLabel`
// (lib/i18n.ts STRINGS). Money line labels and authority names come
// from the server and stay as written.

/**
 * @typedef {{
 *   eyebrow: string,
 *   signInHeading: string,
 *   receiptsInAccount: string,
 *   signIn: string,
 *   toContinue: string,
 *   errorHeading: string,
 *   errorBody: string,
 *   loading: string,
 *   emptyTitle: string,
 *   emptyBody: string,
 *   startOrder: string,
 *   count: (n: number) => string,
 *   paidSum: (amount: string) => string,
 *   testMode: string,
 *   paidOn: (date: string) => string,
 *   orderedOn: (date: string) => string,
 *   viewReceipt: string
 * }} BillingCopy
 */

/** @type {Record<'en' | 'es' | 'fr', BillingCopy>} */
export const BILLING_COPY = {
	en: {
		eyebrow: 'Your account',
		signInHeading: 'Sign in to see your receipts.',
		receiptsInAccount: 'Receipts live in your account.',
		signIn: 'Sign in',
		toContinue: 'to continue.',
		errorHeading: 'Receipts are unavailable right now.',
		errorBody: 'Try again in a bit. Your orders and payments are safe.',
		loading: 'Loading receipts',
		emptyTitle: 'No receipts yet.',
		emptyBody: 'Paid orders show their receipt here, line by line.',
		startOrder: 'Start an order',
		count: (n) => `${n} ${n === 1 ? 'receipt' : 'receipts'}`,
		paidSum: (amount) => `${amount} paid`,
		testMode: 'Test mode',
		paidOn: (date) => `Paid ${date}`,
		orderedOn: (date) => `Ordered ${date}`,
		viewReceipt: 'View receipt'
	},
	es: {
		eyebrow: 'Su cuenta',
		signInHeading: 'Inicie sesión para ver sus recibos.',
		receiptsInAccount: 'Los recibos están en su cuenta.',
		signIn: 'Inicie sesión',
		toContinue: 'para continuar.',
		errorHeading: 'Los recibos no están disponibles en este momento.',
		errorBody: 'Inténtelo de nuevo en un rato. Sus pedidos y pagos están a salvo.',
		loading: 'Cargando recibos',
		emptyTitle: 'Aún no hay recibos.',
		emptyBody: 'Los pedidos pagados muestran aquí su recibo, línea por línea.',
		startOrder: 'Empezar un pedido',
		count: (n) => `${n} ${n === 1 ? 'recibo' : 'recibos'}`,
		paidSum: (amount) => `${amount} pagados`,
		testMode: 'Modo de prueba',
		paidOn: (date) => `Pagado el ${date}`,
		orderedOn: (date) => `Pedido el ${date}`,
		viewReceipt: 'Ver recibo'
	},
	fr: {
		eyebrow: 'Votre compte',
		signInHeading: 'Connectez-vous pour voir vos reçus.',
		receiptsInAccount: 'Vos reçus se trouvent dans votre compte.',
		signIn: 'Connectez-vous',
		toContinue: 'pour continuer.',
		errorHeading: 'Les reçus sont indisponibles pour le moment.',
		errorBody: 'Réessayez dans un instant. Vos commandes et paiements sont en sécurité.',
		loading: 'Chargement des reçus',
		emptyTitle: 'Aucun reçu pour l’instant.',
		emptyBody: 'Les commandes payées affichent ici leur reçu, ligne par ligne.',
		startOrder: 'Commencer une commande',
		count: (n) => `${n} ${n === 1 ? 'reçu' : 'reçus'}`,
		paidSum: (amount) => `${amount} payés`,
		testMode: 'Mode test',
		paidOn: (date) => `Payé le ${date}`,
		orderedOn: (date) => `Commandé le ${date}`,
		viewReceipt: 'Voir le reçu'
	}
};
