// The chat's consent flow for items marked "ask" (docs/contracts.md, POST /api/chat):
// when the chosen backend is not local and the context holds `ask` items that are not
// in `approved`, the stream sends `consent_required { ids, chat_id }` and ends without
// sending anything. The ui shows those items; on approval it resends the same body with
// the ids in `approved`. Pure state, so it is unit-tested in node.
//
// Approvals are remembered for the rest of the chat (a new chat starts empty). Deny never
// resends to the same backend: the server would only ask again. It can resend to a local
// backend instead (nothing leaves the machine there), or just stop.

export function createConsent() {
  return { chatId: null, approved: new Set(), pending: null };
}

// A chat id arrived (done or consent_required): a different chat forgets old approvals.
export function setChat(state, chatId) {
  if (!chatId) return state;
  if (state.chatId && state.chatId !== chatId) state.approved = new Set();
  state.chatId = chatId;
  return state;
}

export function resetConsent(state) {
  state.chatId = null; state.approved = new Set(); state.pending = null;
  return state;
}

// The request body for a send, carrying the approvals so far.
export function withApprovals(state, body) {
  const out = { ...body };
  if (state.chatId && !out.chat_id) out.chat_id = state.chatId;
  const approved = [...new Set([...(body.approved || []), ...state.approved])];
  if (approved.length) out.approved = approved; else delete out.approved;
  return out;
}

// Handle a consent_required event for the body that was sent. Returns
//   { kind: 'ask', ids }      — show these to the user
//   { kind: 'loop', ids }     — the server asked again for ids already approved: stop
export function onConsentRequired(state, body, data) {
  const ids = [...new Set((data && data.ids) || [])];
  setChat(state, data && data.chat_id);
  const sent = new Set(body.approved || []);
  if (ids.length && ids.every((id) => sent.has(id))) { state.pending = null; return { kind: 'loop', ids }; }
  state.pending = { ids, body: { ...body, ...(state.chatId ? { chat_id: state.chatId } : {}) } };
  return { kind: 'ask', ids };
}

// The user agreed: remember the ids for this chat and return the body to resend.
export function approve(state) {
  const p = state.pending;
  if (!p) return null;
  for (const id of p.ids) state.approved.add(id);
  state.pending = null;
  return withApprovals(state, p.body);
}

// The user said no. With a local backend id, returns a body that asks it instead
// (nothing leaves the machine, so no approval is needed); otherwise null (just stop).
export function deny(state, localBackendId) {
  const p = state.pending;
  state.pending = null;
  if (!p || !localBackendId) return null;
  return withApprovals(state, { ...p.body, backend: localBackendId });
}
