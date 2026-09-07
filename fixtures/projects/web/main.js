let count = 0;
document.querySelector('#increment').onclick = () => document.querySelector('#count').textContent = String(++count);
document.querySelector('#crash').onclick = () => { throw new Error('P11 deliberate frontend runtime failure'); };
