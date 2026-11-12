// the desktop shell calls this (through evaluate_script) when the server cannot start
window.anamnesisSplashError = function (message) {
  const el = document.getElementById('state');
  el.textContent = 'the brain could not wake.\n\n' + message;
  el.classList.add('error');
  document.querySelector('.mark').style.animation = 'none';
};
