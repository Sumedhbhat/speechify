speechify.onState((s) => {
  document.getElementById("indicator").textContent = s.status;
});
