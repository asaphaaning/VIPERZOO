'use strict';

if (Process.platform !== 'windows') {
  throw new Error(`VIPERZOO NexusTK tap requires Windows, got ${Process.platform}`);
}

if (Process.arch !== 'ia32') {
  throw new Error(`VIPERZOO NexusTK 752 tap requires ia32, got ${Process.arch}`);
}

const mainModule = Process.mainModule;

function resolveHook(name, rva) {
  const target = mainModule.base.add(rva);
  const moduleEnd = mainModule.base.add(mainModule.size);

  if (target.compare(mainModule.base) < 0 || target.compare(moduleEnd) >= 0) {
    throw new Error(`${name} RVA 0x${rva.toString(16)} is outside ${mainModule.name}`);
  }

  const range = Process.findRangeByAddress(target);

  if (range === null || !range.protection.includes('x')) {
    throw new Error(`${name} address ${target} is not executable`);
  }

  return target;
}

function emitPacket(direction, address, length, threadId) {
  if (address.isNull()) {
    send({ type: 'warning', message: `${direction} packet pointer is null` });
    return;
  }

  if (length < 0 || length > CONFIG.maxPacketSize) {
    send({ type: 'warning', message: `${direction} packet length ${length} rejected` });
    return;
  }

  try {
    send({ type: 'packet', direction, length, threadId }, address.readByteArray(length));
  } catch (error) {
    send({ type: 'warning', message: `${direction} packet read failed: ${error}` });
  }
}

function readClientResources() {
  // NexusTK 752's incoming 0x08 receiver (RVA 0x1bc420) stores these
  // pools on a persistent model published through RVA 0x29b4e4. The vtable
  // guard makes this build-specific fallback fail closed after client changes.
  const singletonSlot = mainModule.base.add(0x0029b4e4);
  const expectedVtable = mainModule.base.add(0x00230cb4);

  try {
    const model = singletonSlot.readPointer();

    if (model.isNull()) {
      return { state: 'unknown', reason: 'client resource model is not initialized' };
    }

    const range = Process.findRangeByAddress(model);

    if (range === null || !range.protection.includes('r') ||
        model.add(0x110).compare(range.base.add(range.size)) >= 0) {
      return { state: 'unknown', reason: 'client resource model is not fully readable' };
    }

    const vtable = model.readPointer();

    if (!vtable.equals(expectedVtable)) {
      return { state: 'unknown', reason: `client resource model vtable mismatch (${vtable})` };
    }

    const vita = model.add(0x104).readU32();
    const maxVita = model.add(0x108).readU32();
    const mana = model.add(0x10c).readU32();
    const maxMana = model.add(0x110).readU32();

    if (maxVita === 0 || maxMana === 0 || vita > maxVita || mana > maxMana) {
      return {
        state: 'unknown',
        reason: `client resource values failed validation (${vita}/${maxVita}, ${mana}/${maxMana})`
      };
    }

    return {
      state: 'known',
      vita,
      max_vita: maxVita,
      mana,
      max_mana: maxMana,
      source: 'client-memory-build-752'
    };
  } catch (error) {
    return { state: 'unknown', reason: `client resource projection read failed: ${error}` };
  }
}

function readClientMapContext() {
  // Build 752 keeps the active map on the object published through RVA
  // 0x27a764. The incoming `0x15` handler (RVA 0x1104d0, reached from the
  // dispatcher at RVA 0x107c90) writes identity and dimensions together:
  //
  //   mov [esi+0x3F2], bx   ; map identity
  //   mov [esi+0x3F4], di   ; width
  //   mov [esi+0x3F6], ax   ; height
  //
  // and guards the reload by comparing all three, which is what makes the
  // triple a checkable unit rather than a bare number. Every caller of the
  // accessor at RVA 0x109470 loads the object from that one slot, so the
  // path below is the client's own. The accessor bytes are verified first,
  // so a changed executable yields Unknown instead of a wrong map.
  const accessor = mainModule.base.add(0x00109470);
  const accessorSignature = [0x0f, 0xb7, 0x81, 0xf2, 0x03, 0x00, 0x00, 0xc3];

  try {
    if (!matchesBytes(accessor, accessorSignature)) {
      return { state: 'unknown', reason: 'client map accessor signature mismatch' };
    }

    const model = mainModule.base.add(0x0027a764).readPointer();

    if (model.isNull()) {
      return { state: 'unknown', reason: 'client map model is not initialized' };
    }

    const range = Process.findRangeByAddress(model);

    if (range === null || !range.protection.includes('r') ||
        model.add(0x3f8).compare(range.base.add(range.size)) >= 0) {
      return { state: 'unknown', reason: 'client map model is not fully readable' };
    }

    const id = model.add(0x3f2).readU16();
    const width = model.add(0x3f4).readU16();
    const height = model.add(0x3f6).readU16();

    // A map the client can render has both extents; zero means the `0x15`
    // handler has not run yet in this client session.
    if (id === 0 || width === 0 || height === 0 || width > 512 || height > 512) {
      return {
        state: 'unknown',
        reason: `client map values failed validation (${id}, ${width}x${height})`
      };
    }

    // The `0x15` handler copies the map name into a stack local and formats it
    // for display; it never stores it on the map model, which is why the title
    // is not beside the identity above. It reaches a separate object published
    // through RVA 0x29b4b4 — the same singleton bank as the resource model at
    // 0x29b4e4 — whose title field sits at +0xf8. Absent rather than guessed:
    // anything unreadable or implausible yields no title at all.
    let title = null;

    try {
      const owner = mainModule.base.add(0x0029b4b4).readPointer();

      if (!owner.isNull()) {
        const range = Process.findRangeByAddress(owner);

        if (range !== null && range.protection.includes('r') &&
            owner.add(0xf8 + 2).compare(range.base.add(range.size)) < 0) {
          const text = owner.add(0xf8).readUtf16String();

          if (typeof text === 'string' && text.length > 0 && text.length <= 64 &&
              /^[\x20-\x7e]+$/.test(text)) {
            title = text;
          }
        }
      }
    } catch (error) {
      title = null;
    }

    return { state: 'known', id, width, height, title, source: 'client-memory-build-752' };
  } catch (error) {
    return { state: 'unknown', reason: `client map projection read failed: ${error}` };
  }
}

/// Reports readable text reachable from the map model, for offset discovery.
///
/// The `0x15` handler copies the map name into a stack local and formats it for
/// display; no store onto the map model appears in it. If the title lives there
/// at all it is behind a pointer, the way MFC keeps a `CString`, so a scan for
/// inline text alone would miss it. This looks for both and reports what it
/// finds rather than deciding anything — the caller compares the result with
/// the map the server actually named.
function probeClientMapStrings(limit) {
  const accessor = mainModule.base.add(0x00109470);
  const accessorSignature = [0x0f, 0xb7, 0x81, 0xf2, 0x03, 0x00, 0x00, 0xc3];

  try {
    if (!matchesBytes(accessor, accessorSignature)) {
      return { state: 'unknown', reason: 'client map accessor signature mismatch' };
    }

    const model = mainModule.base.add(0x0027a764).readPointer();

    if (model.isNull()) {
      return { state: 'unknown', reason: 'client map model is not initialized' };
    }

    const span = Number(limit) || 0x1000;
    const found = [];

    const plausible = (text) =>
      typeof text === 'string' &&
      text.length >= 3 &&
      text.length <= 64 &&
      /^[\x20-\x7e]+$/.test(text);

    for (let offset = 0; offset + 4 <= span; offset += 4) {
      const at = model.add(offset);

      // Inline wide text, as a fixed-size character array would hold it.
      try {
        const inline = at.readUtf16String();
        if (plausible(inline)) found.push({ offset, kind: 'inline', text: inline });
      } catch (error) { /* unreadable is simply not a candidate */ }

      // Text behind a pointer, as a CString-style member would hold it.
      try {
        const pointer = at.readPointer();
        if (!pointer.isNull()) {
          const range = Process.findRangeByAddress(pointer);
          if (range !== null && range.protection.includes('r')) {
            const wide = pointer.readUtf16String();
            if (plausible(wide)) found.push({ offset, kind: 'pointer', text: wide });
            const narrow = pointer.readAnsiString();
            if (plausible(narrow)) found.push({ offset, kind: 'pointer-ansi', text: narrow });
          }
        }
      } catch (error) { /* unreadable is simply not a candidate */ }
    }

    return { state: 'known', found, source: 'client-memory-build-752' };
  } catch (error) {
    return { state: 'unknown', reason: `client map string probe failed: ${error}` };
  }
}

/// Locates wide text in the client and reports any static pointer to it.
///
/// A title is worth far more than a bare number as evidence: two bytes can
/// equal a map id by chance, but a run of characters spelling the current map
/// name effectively cannot. This reports where that text lives and whether any
/// module-owned slot points at it, which is what a warm-attach read would need
/// — an address that survives a restart, not one that happens to be valid now.
function probeClientText(text) {
  const needle = [];

  for (const character of String(text)) {
    const code = character.charCodeAt(0);
    needle.push(code & 0xff, (code >> 8) & 0xff);
  }

  const pattern = needle.map((byte) => byte.toString(16).padStart(2, '0')).join(' ');
  const main = Process.enumerateModules()[0];
  const hits = [];

  for (const range of Process.enumerateRanges('r--')) {
    let matches;
    try {
      matches = Memory.scanSync(range.base, range.size, pattern);
    } catch (error) {
      continue;
    }

    for (const match of matches) {
      const inMain =
        match.address.compare(main.base) >= 0 &&
        match.address.compare(main.base.add(main.size)) < 0;
      const record = {
        address: match.address.toString(),
        in_main_module: inMain,
        module_offset: inMain ? match.address.sub(main.base).toInt32() : null,
        referenced_from: []
      };

      // A heap address is only useful if something stable points at it. Search
      // every writable range, not just module-owned ones: a title held by a
      // heap object is reachable if that object is, so the holder's identity
      // matters as much as whether the module points at the text directly.
      if (!inMain) {
        const target = match.address;
        const model = mainModule.base.add(0x0027a764).readPointer();

        for (const candidate of Process.enumerateRanges('rw-')) {
          let bytes;
          try {
            bytes = new Uint8Array(candidate.base.readByteArray(candidate.size));
          } catch (error) {
            continue;
          }

          // Match a window rather than the exact address: text embedded in a
          // larger allocation is referenced through the container's base, so
          // an exact match finds nothing even when the object is reachable.
          const wanted = target.toUInt32();
          const floor = wanted - 0x200;

          for (let offset = 0; offset + 4 <= bytes.length; offset += 4) {
            const value =
              bytes[offset] | (bytes[offset + 1] << 8) |
              (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24);
            const pointer = value >>> 0;

            if (pointer <= wanted && pointer >= floor) {
              const holder = candidate.base.add(offset);
              const inModule =
                holder.compare(main.base) >= 0 &&
                holder.compare(main.base.add(main.size)) < 0;

              const delta = wanted - pointer;
              const where = inModule
                ? `module+0x${holder.sub(main.base).toInt32().toString(16)}`
                : (!model.isNull() && holder.compare(model) >= 0 &&
                   holder.sub(model).toInt32() < 0x8000)
                  ? `mapmodel+0x${holder.sub(model).toInt32().toString(16)}`
                  : `heap ${holder}`;

              record.referenced_from.push(`${where} (text at +0x${delta.toString(16)})`);

              if (record.referenced_from.length >= 32) break;
            }
          }
        }
      }

      hits.push(record);
      if (hits.length >= 24) return { state: 'known', hits };
    }
  }

  return { state: 'known', hits };
}

function matchesBytes(address, expected) {
  const actual = new Uint8Array(address.readByteArray(expected.length));

  return expected.every((value, index) => actual[index] === value);
}

function canonicalInventoryName(displayName) {
  // Captured 0x0F rows distinguish labels such as `Ginko wood (30)` and
  // `Rabbit meat (33)` from their canonical names. The client model stores the
  // former, so remove only the same terminal numeric-label convention.
  return displayName.replace(/ \(\d+\)$/, '');
}

function readClientInventory() {
  // InventoryPane2 reaches every build-752 carried slot through this model:
  //
  //   *(base + 0x27a748) + 0x133f08 + slot * 0x1fc
  //
  // The accessor signature is checked before any model field is trusted. A
  // changed executable therefore turns this optional seed into Unknown instead
  // of projecting an incorrect complete inventory.
  const accessor = mainModule.base.add(0x001a3870);
  const accessorSignature = [
    0x55, 0x8b, 0xec, 0x0f, 0xbe, 0x45, 0x08, 0x81,
    0xc1, 0x08, 0x3f, 0x13, 0x00, 0x69, 0xc0, 0xfc,
    0x01, 0x00, 0x00, 0x03, 0xc1, 0x5d, 0xc2, 0x04,
    0x00
  ];

  try {
    if (!matchesBytes(accessor, accessorSignature)) {
      return { state: 'unknown', reason: 'client inventory accessor signature mismatch' };
    }

    const model = mainModule.base.add(0x0027a748).readPointer();
    const capacityModel = mainModule.base.add(0x0029ae0c).readPointer();

    if (model.isNull() || capacityModel.isNull()) {
      return { state: 'unknown', reason: 'client inventory model is not initialized' };
    }

    const capacity = capacityModel.add(0x284).readU8();

    if (capacity < 1 || capacity > 52) {
      return {
        state: 'unknown',
        reason: `client inventory capacity failed validation (${capacity})`
      };
    }

    const items = [];

    for (let slot = 1; slot <= capacity; slot += 1) {
      const record = model.add(0x133f08 + slot * 0x1fc);
      const range = Process.findRangeByAddress(record);

      if (range === null || !range.protection.includes('r') ||
          record.add(0x1ec).compare(range.base.add(range.size)) >= 0) {
        return {
          state: 'unknown',
          reason: `client inventory slot ${slot} is not fully readable`
        };
      }

      if (record.readU8() === 0) {
        continue;
      }

      const name = record.add(6).readUtf16String();
      const amount = record.add(0x1e8).readU32();

      if (name === null || name.trim().length === 0 || amount === 0) {
        return {
          state: 'unknown',
          reason: `client inventory slot ${slot} failed occupied-record validation`
        };
      }

      items.push({
        slot,
        icon_id: record.add(2).readU16(),
        icon_color: record.add(4).readU8(),
        name: canonicalInventoryName(name),
        amount
      });
    }

    return {
      state: 'known',
      capacity,
      items,
      source: 'client-memory-build-752'
    };
  } catch (error) {
    return { state: 'unknown', reason: `client inventory projection read failed: ${error}` };
  }
}

const clientKeyInfo = {
  up: { vk: 0x26, scan: 0x48 },
  right: { vk: 0x27, scan: 0x4d },
  down: { vk: 0x28, scan: 0x50 },
  left: { vk: 0x25, scan: 0x4b }
};
const clientLetterScanCodes = [
  0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32,
  0x31, 0x18, 0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c
];
let clientStepState = null;
let clientCastState = null;
let clientInventoryState = null;
let forceMapDataEnabled = false;
let outgoingSession = null;
let sessionClosing = false;
let pendingPlaintextBodies = [];
let pendingTravelSelection = null;
let combatWakeDirection = null;
let directPlaintextSendDepth = 0;
const MAX_PENDING_PLAINTEXT_BODIES = 32;
const PENDING_PLAINTEXT_POLL_RETRY_MS = 220;
const MAX_PENDING_PLAINTEXT_WAKE_ATTEMPTS = 10;
// Two independent successful bank captures waited about 1.22 seconds between
// each matching 0x2f response and the next client selection. Sending 0x4c in
// the same scheduling turn as the quantity prompt was observed in ginko-end21
// and was ignored by the server despite having the correct body grammar.
const DIALOG_RESPONSE_SETTLE_MS = 1200;
const CLIENT_STEP_RETRY_MS = 220;
const MAX_CLIENT_STEP_TAP_ATTEMPTS = 10;
const CLIENT_SOCKET_OFFSET = 0x001d578;
const WSOCK32_SEND_IAT_RVA = 0x0020d518;
const WSOCK32_CLOSESOCKET_IAT_RVA = 0x0020d538;
const WSOCK32_RECV_IAT_RVA = 0x0020d53c;
const CLIENT_IDLE_WATCHER_SLOT_RVA = 0x0029af1c;
const CLIENT_IDLE_WATCHER_VTABLE_RVA = 0x0021b56c;
const CLIENT_IDLE_RESET_RVA = 0x000cfef0;
const TRAVEL_SELECTOR_SUBMIT_RVA = 0x001c3630;
const TRAVEL_SELECTOR_CONSTRUCT_RVA = 0x001c2ac0;
const TRAVEL_SELECTOR_SLOT_RVA = 0x0029b454;
const TRAVEL_SELECTOR_VTABLE_RVA = 0x00231400;
const TRAVEL_SELECTOR_ENTRY_SIZE = 0x94;
const MAX_TRAVEL_SELECTOR_ENTRIES = 64;
let submitTravelSelectorRow = null;

function activeGameSocket() {
  if (sessionClosing || outgoingSession === null || outgoingSession.isNull()) {
    return null;
  }

  const socket = outgoingSession.add(CLIENT_SOCKET_OFFSET).readS32();
  return socket === -1 ? null : socket;
}

function reportTransportClosed(source) {
  if (sessionClosing) {
    return;
  }

  sessionClosing = true;
  outgoingSession = null;
  pendingPlaintextBodies = [];
  send({ type: 'transport-closed', source });
}

function reportTransportFault(operation, code) {
  send({ type: 'transport-fault', operation, code });
}

function isFatalSocketError(code) {
  return [
    10050, // WSAENETDOWN
    10051, // WSAENETUNREACH
    10052, // WSAENETRESET
    10053, // WSAECONNABORTED
    10054, // WSAECONNRESET
    10057, // WSAENOTCONN
    10058, // WSAESHUTDOWN
    10060, // WSAETIMEDOUT
    10061  // WSAECONNREFUSED
  ].includes(code);
}

function importedFunction(name, iatRva) {
  const address = mainModule.base.add(iatRva).readPointer();
  const range = Process.findRangeByAddress(address);

  if (address.isNull() || range === null || !range.protection.includes('x')) {
    throw new Error(`${name} import target ${address} is not executable`);
  }

  return address;
}

function installTransportHooks() {
  const sendSocket = importedFunction('WSOCK32!send', WSOCK32_SEND_IAT_RVA);
  const closesocket = importedFunction('WSOCK32!closesocket', WSOCK32_CLOSESOCKET_IAT_RVA);
  const recv = importedFunction('WSOCK32!recv', WSOCK32_RECV_IAT_RVA);

  Interceptor.attach(recv, {
    onEnter(args) {
      try {
        this.gameSocket = activeGameSocket();
      } catch (error) {
        this.gameSocket = null;
      }
      this.socket = args[0].toInt32();
    },
    onLeave(result) {
      if (this.gameSocket === null || this.socket !== this.gameSocket) {
        return;
      }

      const received = result.toInt32();

      if (received === 0) {
        // NexusTK's receive routine branches directly from recv == 0 to
        // closesocket. Observe the orderly remote shutdown at the earliest
        // structural boundary instead of depending on a forwarded Winsock
        // export chosen by the loader.
        reportTransportClosed('recv-zero');
      } else if (received === -1) {
        // Frida snapshots Windows' thread-local last-error value on the
        // invocation context. Calling WSAGetLastError from inside this hook
        // would itself cross a native-call boundary and can yield zero instead
        // of the error belonging to this recv.
        const code = this.lastError;

        // WSAEWOULDBLOCK is the ordinary result for this non-blocking socket.
        // Preserve every other failure at the evidence boundary.
        if (code !== 10035) {
          reportTransportFault('receive', code);

          if (isFatalSocketError(code)) {
            reportTransportClosed(`recv-error-${code}`);
          }
        }
      }
    }
  });

  Interceptor.attach(sendSocket, {
    onEnter(args) {
      try {
        this.gameSocket = activeGameSocket();
      } catch (error) {
        this.gameSocket = null;
      }
      this.socket = args[0].toInt32();
    },
    onLeave(result) {
      if (this.gameSocket === null || this.socket !== this.gameSocket || result.toInt32() !== -1) {
        return;
      }

      const code = this.lastError;

      if (code !== 10035) {
        reportTransportFault('send', code);

        if (isFatalSocketError(code)) {
          reportTransportClosed(`send-error-${code}`);
        }
      }
    }
  });

  Interceptor.attach(closesocket, {
    onEnter(args) {
      try {
        const activeSocket = activeGameSocket();
        const closingSocket = args[0].toInt32();

        if (activeSocket !== null && closingSocket === activeSocket) {
          // NexusTK 752 keeps its game socket at +0x1D578. Its receive path
          // calls closesocket directly when recv returns zero, so this is the
          // structural boundary for a remote orderly shutdown that produces
          // no plaintext 0x0B body.
          reportTransportClosed('closesocket');
        }
      } catch (error) {
        send({ type: 'warning', message: `transport close correlation failed: ${error}` });
      }
    }
  });
}

function invokeClientActivity() {
  // Build 752 calls this method from both accepted keyboard and mouse input
  // paths. It clears +0xF8 and rearms a local 20-second timer. This is retained
  // as an explicit research RPC, but is not session maintenance: ginko-end9
  // proved repeated resets do not prevent the server-side disconnect.
  const reset = mainModule.base.add(CLIENT_IDLE_RESET_RVA);
  const signature = [
    0x56, 0x8b, 0xf1, 0x57, 0x8d, 0x8e, 0xa4, 0x00, 0x00,
    0x00, 0xc6, 0x86, 0xf8, 0x00, 0x00, 0x00, 0x00
  ];

  if (!matchesBytes(reset, signature)) {
    throw new Error('client IdleWatcher reset signature mismatch');
  }

  const watcher = mainModule.base.add(CLIENT_IDLE_WATCHER_SLOT_RVA).readPointer();
  const expectedVtable = mainModule.base.add(CLIENT_IDLE_WATCHER_VTABLE_RVA);

  if (watcher.isNull() || !watcher.readPointer().equals(expectedVtable)) {
    throw new Error('client IdleWatcher singleton is unavailable or incompatible');
  }

  const resetIdle = new NativeFunction(reset, 'void', ['pointer'], 'thiscall');
  resetIdle(watcher);
}

function resolveClientWindow() {
  // Build-752 application singleton and main HWND field. This works while the
  // window is unfocused and still leaves packet construction to the client.
  const application = mainModule.base.add(0x0027ab1c).readPointer();

  if (application.isNull()) {
    throw new Error('client application object is not initialized');
  }

  const hwnd = application.add(0x828).readPointer();

  if (hwnd.isNull()) {
    throw new Error('client main window handle is not initialized');
  }

  return hwnd;
}

function postMessage() {
  const address = Module.findGlobalExportByName('PostMessageW');

  if (address === null) {
    throw new Error('PostMessageW export was not found');
  }

  return new NativeFunction(address, 'int', ['pointer', 'uint', 'uint', 'uint']);
}

function invokeClientKeyTap(keyName) {
  const key = clientKeyInfo[keyName];

  if (key === undefined) {
    throw new Error(`unsupported client key ${keyName}`);
  }

  const post = postMessage();
  const hwnd = resolveClientWindow();
  const keyDown = (0x01000001 | (key.scan << 16)) >>> 0;
  const keyUp = (0xc1000001 | (key.scan << 16)) >>> 0;

  if (post(hwnd, 0x0100, key.vk, keyDown) === 0) {
    throw new Error('PostMessageW(WM_KEYDOWN) failed');
  }

  setTimeout(() => {
    try {
      if (post(hwnd, 0x0101, key.vk, keyUp) === 0) {
        throw new Error('PostMessageW(WM_KEYUP) failed');
      }
    } catch (error) {
      send({ type: 'client-action-failed', action: 'step', error: String(error) });
    }
  }, 60);
}

function invokeVirtualKeyTap(vk, scan, action) {
  const post = postMessage();
  const hwnd = resolveClientWindow();
  const keyDown = (0x01000001 | (scan << 16)) >>> 0;
  const keyUp = (0xc1000001 | (scan << 16)) >>> 0;

  if (post(hwnd, 0x0100, vk, keyDown) === 0) {
    throw new Error(`PostMessageW(${action} down) failed`);
  }

  setTimeout(() => {
    try {
      if (post(hwnd, 0x0101, vk, keyUp) === 0) {
        throw new Error(`PostMessageW(${action} up) failed`);
      }
    } catch (error) {
      send({ type: 'client-action-failed', action, error: String(error) });
    }
  }, 60);
}

function directionKey(direction) {
  const value = Number(direction);

  if (!Number.isInteger(value) || value < 0 || value > 3) {
    throw new Error(`direction must be between 0 and 3 (received ${direction})`);
  }

  return clientKeyInfo[['up', 'right', 'down', 'left'][value]];
}

function directionName(direction) {
  const value = Number(direction);
  directionKey(value);
  return ['up', 'right', 'down', 'left'][value];
}

function invokeClientAttack(direction) {
  const wakeDirection = directionName(direction);
  combatWakeDirection = wakeDirection;

  // The direction transition is a deterministic way to make NexusTK's
  // separate game-network thread run. If it natively emits 0x13, the outgoing
  // hook consumes the queued duplicate and that native body is the semantic
  // attack. Otherwise the completed native body gives `flushPlaintextBody`
  // the client-owned boundary on which to submit the queued 0x13.
  return invokeCombatPlaintextBody([0x13, 0x00, 0x00], 'attack', wakeDirection);
}

function invokeClientPickup() {
  return invokePlaintextBody([0x07, 0x01, 0x00], 'pickup');
}

function invokeClientFace(direction) {
  const value = Number(direction);
  combatWakeDirection = directionName(value);
  return invokeCombatPlaintextBody([0x11, value, 0x00], 'face', combatWakeDirection);
}

function invokeClientUseInventory(slot) {
  const numericSlot = Number(slot);

  if (!Number.isInteger(numericSlot) || numericSlot < 1 || numericSlot > 26) {
    throw new Error(`inventory slot must be between 1 and 26 (received ${slot})`);
  }

  if (clientInventoryState !== null && clientInventoryState.phase !== 'complete') {
    throw new Error('a client inventory activation is already in flight');
  }

  const state = { slot: numericSlot, phase: 'posting-hotkey' };
  clientInventoryState = state;
  invokeVirtualKeyTap(0x55, 0x16, 'use-inventory');

  setTimeout(() => {
    try {
      const letterVk = 0x41 + numericSlot - 1;
      const letterScan = clientLetterScanCodes[numericSlot - 1];
      invokeVirtualKeyTap(letterVk, letterScan, 'use-inventory-slot');
      state.phase = 'awaiting-outgoing-activation';

      setTimeout(() => {
        if (clientInventoryState === state && state.phase !== 'complete') {
          state.phase = 'complete';
        }
      }, 1500);
    } catch (error) {
      state.phase = 'complete';
      send({ type: 'client-action-failed', action: 'use-inventory', error: String(error) });
    }
  }, 120);
}

function noteClientInventoryOutbound(input, length) {
  const state = clientInventoryState;

  if (state === null || state.phase === 'complete' || length < 2) {
    return;
  }

  if (input.readU8() === 0x1c && input.add(1).readU8() === state.slot) {
    state.phase = 'complete';
  }
}

function invokePlaintextBody(bytes, action) {
  return enqueuePlaintextBodies([bytes], action, requestNetworkPoll);
}

function invokeCombatPlaintextBody(bytes, action, direction) {
  return enqueuePlaintextBodies([bytes], action, () => invokeClientKeyTap(direction));
}

function enqueuePlaintextBodies(bodies, action, wakeNetworkThread, dialogEntity = null) {
  if (!Array.isArray(bodies) || bodies.length === 0) {
    throw new Error(`${action} requires at least one plaintext body`);
  }

  if (sessionClosing) {
    throw new Error(`${action} rejected because the client session is closing`);
  }

  if (outgoingSession === null || outgoingSession.isNull()) {
    throw new Error(`${action} requires one observed live outgoing session`);
  }

  if (pendingPlaintextBodies.length + bodies.length > MAX_PENDING_PLAINTEXT_BODIES) {
    throw new Error(`${action} rejected because the client-thread action queue is full`);
  }

  // Frida RPC executes on an agent-owned thread. Calling the client sender
  // there can race the network thread's heartbeat and corrupt shared sequence
  // or cipher state after the plaintext hook. Queue the logical body instead;
  // the build-752 network-poll hook drains it on the client-owned network
  // thread after that poll cycle has completed.
  return new Promise((resolve, reject) => {
    const batch = {
      action,
      dialogEntity,
      wakeNetworkThread,
      remaining: bodies.length,
      settled: false,
      resolve,
      reject,
    };
    const pending = bodies.map((bytes, index) => ({
      bytes: Array.from(bytes),
      action,
      batch,
      ready: index === 0,
      readyAfter: null,
      waitForDialogResponse: dialogEntity !== null && index >= 1,
      wakeAttempts: 1,
    }));
    pendingPlaintextBodies.push(...pending);

    try {
      wakeNetworkThread();
    } catch (error) {
      failPendingPlaintextBatch(pending[0], error);
      return;
    }

    // A native direction may be suppressed during NexusTK's short
    // collision/combat window. Retry only while this exact body remains queued.
    // The outgoing hook either consumes a semantically equivalent native 0x13
    // or drains one logical body, so repeated wakes cannot duplicate the intent.
    schedulePendingPlaintextWake(pending[0], wakeNetworkThread);
  });
}

function completePendingPlaintextBody(pending) {
  const batch = pending.batch;

  if (batch.settled) {
    return;
  }

  batch.remaining -= 1;

  if (batch.remaining === 0) {
    batch.settled = true;
    batch.resolve(true);
  }
}

function failPendingPlaintextBatch(pending, error) {
  const batch = pending.batch;
  pendingPlaintextBodies = pendingPlaintextBodies.filter(
    candidate => candidate.batch !== batch
  );

  if (!batch.settled) {
    batch.settled = true;
    batch.reject(new Error(`${batch.action} dispatch failed: ${error}`));
  }
}

function failAllPendingPlaintextBatches(error) {
  const pending = pendingPlaintextBodies;
  pendingPlaintextBodies = [];
  const failed = new Set();

  for (const body of pending) {
    if (!failed.has(body.batch)) {
      failed.add(body.batch);

      if (!body.batch.settled) {
        body.batch.settled = true;
        body.batch.reject(new Error(`${body.batch.action} dispatch failed: ${error}`));
      }
    }
  }
}

function schedulePendingPlaintextWake(pending, wakeNetworkThread) {
  setTimeout(() => {
    if (sessionClosing || !pendingPlaintextBodies.includes(pending)) {
      return;
    }

    if (pending.wakeAttempts >= MAX_PENDING_PLAINTEXT_WAKE_ATTEMPTS) {
      send({
        type: 'warning',
        message: `client action ${pending.action} remains queued after ${pending.wakeAttempts} network wakes; awaiting the next client-owned boundary`
      });
      return;
    }

    pending.wakeAttempts += 1;

    try {
      wakeNetworkThread();
    } catch (error) {
      send({
        type: 'client-action-failed',
        action: pending.action,
        error: `pending action poll retry failed: ${error}`
      });
    }

    schedulePendingPlaintextWake(pending, wakeNetworkThread);
  }, PENDING_PLAINTEXT_POLL_RETRY_MS);
}

function satisfyPendingNativeBody(opcode) {
  const pending = pendingPlaintextBodies[0];

  if (opcode === 0x13 && pending !== undefined && pending.action === 'attack') {
    pendingPlaintextBodies.shift();
    completePendingPlaintextBody(pending);
  }
}

function requestNetworkPoll() {
  // Direct logical actions are queued for the client-owned network thread. A
  // quiet client can otherwise stay between UI messages after a portal and
  // leave the final action pending indefinitely. A synthetic Control tap is
  // accepted input with no gameplay command: unlike WM_NULL, it advances the
  // client's input loop without changing map state or pretending to be
  // liveness maintenance.
  const post = postMessage();
  const hwnd = resolveClientWindow();
  const scan = 0x1d;
  const keyDown = (0x01000001 | (scan << 16)) >>> 0;
  const keyUp = (0xc1000001 | (scan << 16)) >>> 0;

  if (post(hwnd, 0x0100, 0x11, keyDown) === 0) {
    throw new Error('PostMessageW(Control wake down) failed');
  }

  setTimeout(() => {
    try {
      if (post(hwnd, 0x0101, 0x11, keyUp) === 0) {
        throw new Error('PostMessageW(Control wake up) failed');
      }
    } catch (error) {
      send({ type: 'client-action-failed', action: 'network-wake', error: String(error) });
    }
  }, 30);
}

function flushPlaintextBody(session, threadId) {
  if (sessionClosing || pendingPlaintextBodies.length === 0 || session.isNull()) {
    return null;
  }

  const pending = pendingPlaintextBodies[0];

  if (!pending.ready) {
    return null;
  }

  pendingPlaintextBodies.shift();
  try {
    outgoingSession = session;
    const body = Memory.alloc(pending.bytes.length);
    body.writeByteArray(pending.bytes);

    // The client-owned sender can be invoked from an Interceptor callback.
    // Frida deliberately suppresses a nested callback in that case, so the
    // ordinary outgoing tap does not see this body. Mark this narrow interval
    // and publish the logical packet ourselves after the sender returned. The
    // engine therefore observes exactly the body the client accepted, without
    // falsely treating the wake key's obstruction as a crafted action.
    directPlaintextSendDepth += 1;

    try {
      cryptAndSend(session, body, pending.bytes.length);
    } finally {
      directPlaintextSendDepth -= 1;
    }

    emitPacket('outgoing', body, pending.bytes.length, threadId);
    completePendingPlaintextBody(pending);
    return pending;
  } catch (error) {
    failPendingPlaintextBatch(pending, error);
    send({
      type: 'client-action-failed',
      action: pending.action,
      error: `client-thread send failed: ${error}`
    });
    return null;
  }
}

function schedulePlaintextBody(pending) {
  const delay = Math.max(0, pending.readyAfter - Date.now());

  setTimeout(() => {
    if (sessionClosing || !pendingPlaintextBodies.includes(pending)) {
      return;
    }

    pending.ready = true;

    try {
      pending.batch.wakeNetworkThread();
    } catch (error) {
      send({
        type: 'client-action-failed',
        action: pending.action,
        error: `dialog action poll failed: ${error}`
      });
    }

    schedulePendingPlaintextWake(pending, pending.batch.wakeNetworkThread);
  }, delay);
}

function observeDialogResponse(input, length) {
  const pending = pendingPlaintextBodies[0];

  if (pending === undefined ||
      !pending.waitForDialogResponse ||
      pending.batch.dialogEntity === null ||
      length < 7 ||
      input.isNull() ||
      input.readU8() !== 0x2f) {
    return;
  }

  const entity = (
    (input.add(3).readU8() * 0x1000000) +
    (input.add(4).readU8() << 16) +
    (input.add(5).readU8() << 8) +
    input.add(6).readU8()
  ) >>> 0;

  if (entity !== pending.batch.dialogEntity) {
    return;
  }

  pending.waitForDialogResponse = false;
  pending.readyAfter = Date.now() + DIALOG_RESPONSE_SETTLE_MS;
  schedulePlaintextBody(pending);
}

function flushPlaintextBatch(session, threadId) {
  if (sessionClosing || session === null || session.isNull()) {
    return;
  }

  const first = pendingPlaintextBodies[0];

  if (first === undefined) {
    return;
  }

  const completed = flushPlaintextBody(session, threadId);

  if (completed === null) {
    return;
  }

  const next = pendingPlaintextBodies[0];

  if (next?.batch === completed.batch && !next.waitForDialogResponse) {
    schedulePlaintextBody(next);
  }
}

function entityBytes(entity) {
  const value = Number(entity);

  if (!Number.isInteger(value) || value < 0 || value > 0xffffffff) {
    throw new Error(`entity id is outside u32 (${entity})`);
  }

  return [(value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff];
}

function invokeClientInteract(entity) {
  return invokePlaintextBody([0x43, 0x01, ...entityBytes(entity), 0x00], 'interact');
}

function validatedAsciiBytes(value, label) {
  const bytes = Array.from(value, Number);

  if (bytes.length === 0 || bytes.length > 255 || bytes.some(byte => byte < 1 || byte > 0x7f)) {
    throw new Error(`${label} is not validated ASCII`);
  }

  return bytes;
}

function validatedU16(value, label) {
  const numeric = Number(value);

  if (!Number.isInteger(numeric) || numeric < 0 || numeric > 0xffff) {
    throw new Error(`${label} is outside u16 (${value})`);
  }

  return numeric;
}

function u16Bytes(value, label) {
  const numeric = validatedU16(value, label);
  return [(numeric >>> 8) & 0xff, numeric & 0xff];
}

function invokeClientDialog(entity, command, argument, quantity) {
  return invokePlaintextBody(dialogBody(entity, command, argument, quantity), 'dialog');
}

function dialogBody(entity, command, argument, quantity) {
  const token = Number(command);

  if (!Number.isInteger(token) || token < 0 || token > 0xff) {
    throw new Error(`dialog command is outside u8 (${command})`);
  }

  const prefix = [0x39, 0x01, ...entityBytes(entity), 0x00, token];

  if (argument === null || argument === undefined) {
    if (quantity !== null && quantity !== undefined) {
      throw new Error('dialog quantity requires an argument');
    }
    return [...prefix, 0x00];
  }

  const bytes = validatedAsciiBytes(argument, 'dialog argument');
  // Plain text-only dialog selections terminate the item argument. Shop
  // confirmations do not: their captured command-0x4c grammar continues
  // immediately with a length-prefixed ASCII quantity, then one final NUL.
  // Inserting a NUL before that length made Pepper reject `Yellow scroll` as
  // an impossible quantity during ginko-end15.
  const body = [...prefix, bytes.length, ...bytes];

  if (quantity !== null && quantity !== undefined) {
    const numericQuantity = Number(quantity);

    if (!Number.isInteger(numericQuantity) || numericQuantity < 1 || numericQuantity > 255) {
      throw new Error(`dialog quantity is outside 1..=255 (${quantity})`);
    }

    const quantityBytes = Array.from(String(numericQuantity), value => value.charCodeAt(0));
    body.push(quantityBytes.length, ...quantityBytes, 0x00);
  } else {
    body.push(0x00);
  }

  return body;
}

function invokeClientDialogTransaction(entity, selections) {
  if (!Array.isArray(selections) || selections.length === 0) {
    throw new Error('dialog transaction requires at least one selection');
  }

  const interaction = [0x43, 0x01, ...entityBytes(entity), 0x00];
  const bodies = [interaction];

  for (const selection of selections) {
    if (selection === null || typeof selection !== 'object') {
      throw new Error('dialog transaction selection is not an object');
    }

    bodies.push(dialogBody(entity, selection.command, selection.argument, selection.quantity));
  }

  return enqueuePlaintextBodies(
    bodies,
    'dialog-transaction',
    requestNetworkPoll,
    Number(entity) >>> 0
  );
}

function invokeClientSpeak(channel, text) {
  const numericChannel = Number(channel);

  if (!Number.isInteger(numericChannel) || numericChannel < 0 || numericChannel > 0xff) {
    throw new Error(`speech channel is outside u8 (${channel})`);
  }

  const bytes = validatedAsciiBytes(text, 'speech text');
  return invokePlaintextBody([0x0e, numericChannel, bytes.length, ...bytes, 0x00], 'speech');
}

function invokeClientAnsweredSpell(slot, answer) {
  const numericSlot = Number(slot);

  if (!Number.isInteger(numericSlot) || numericSlot < 1 || numericSlot > 26) {
    throw new Error(`spell slot must be between 1 and 26 (received ${slot})`);
  }

  const bytes = validatedAsciiBytes(answer, 'spell answer');
  return invokePlaintextBody([0x0f, numericSlot, ...bytes, 0x00], 'answered-spell');
}

function activeTravelSelector() {
  const selector = mainModule.base.add(TRAVEL_SELECTOR_SLOT_RVA).readPointer();

  if (selector.isNull() ||
      selector.readPointer().compare(mainModule.base.add(TRAVEL_SELECTOR_VTABLE_RVA)) !== 0) {
    return null;
  }

  return selector;
}

function invokeClientTravel(map) {
  // An immediate selection is a guarded warm-attachment recovery. The client
  // publishes the active selector object while the overlay exists and clears
  // its slot during destruction, so stale rows are never treated as open.
  pendingTravelSelection = null;
  const selector = activeTravelSelector();

  if (selector === null) {
    return false;
  }

  stageClientTravelOnMenu(map);
  pendingTravelSelection.menuObserved = true;
  bindPendingTravelSelection(selector);

  if (pendingTravelSelection === null ||
      pendingTravelSelection.selector === null) {
    pendingTravelSelection = null;
    return false;
  }

  return submitPendingTravelSelection();
}

function validatedTravelSelection(map, x, y) {
  return {
    map: validatedU16(map, 'travel map'),
    x: validatedU16(x, 'travel x'),
    y: validatedU16(y, 'travel y')
  };
}

function stageClientTravelSelection(map, x, y, menuObserved) {
  if (sessionClosing) {
    throw new Error('travel-selection rejected because the client session is closing');
  }

  if (outgoingSession === null || outgoingSession.isNull()) {
    throw new Error('travel-selection requires one observed live outgoing session');
  }

  if (pendingTravelSelection !== null) {
    throw new Error('a travel selection is already waiting for a menu');
  }

  const selection = validatedTravelSelection(map, x, y);
  pendingTravelSelection = {
    action: 'travel-selection',
    map: selection.map,
    x: selection.x,
    y: selection.y,
    menuObserved: Boolean(menuObserved),
    selector: null,
    row: null
  };
}

function stageClientTravelOnMenu(map) {
  if (sessionClosing) {
    throw new Error('travel-selection rejected because the client session is closing');
  }

  if (outgoingSession === null || outgoingSession.isNull()) {
    throw new Error('travel-selection requires one observed live outgoing session');
  }

  if (pendingTravelSelection !== null) {
    throw new Error('a travel selection is already waiting for a menu');
  }

  pendingTravelSelection = {
    action: 'travel-selection',
    map: validatedU16(map, 'travel map'),
    x: null,
    y: null,
    menuObserved: false,
    selector: null,
    row: null
  };
}

function observePendingTravelMenu(input, length) {
  if (pendingTravelSelection === null ||
      length === 0 ||
      input.isNull() ||
      input.readU8() !== 0x2e) {
    return;
  }

  pendingTravelSelection.menuObserved = true;
}

function matchingTravelSelectorRow(selector, selection) {
  const begin = selector.add(0x258).readPointer();
  const end = selector.add(0x25c).readPointer();

  if (begin.isNull() || end.isNull()) {
    return null;
  }

  const byteLength = end.sub(begin).toInt32();

  if (byteLength <= 0 ||
      byteLength % TRAVEL_SELECTOR_ENTRY_SIZE !== 0 ||
      byteLength / TRAVEL_SELECTOR_ENTRY_SIZE > MAX_TRAVEL_SELECTOR_ENTRIES) {
    return null;
  }

  const count = byteLength / TRAVEL_SELECTOR_ENTRY_SIZE;

  for (let index = 0; index < count; index += 1) {
    const entry = begin.add(index * TRAVEL_SELECTOR_ENTRY_SIZE);
    const map = entry.add(0x88).readU16();
    const y = entry.add(0x8c).readU32() & 0xffff;
    const x = entry.add(0x90).readU32() & 0xffff;

    const coordinatesMatch =
      selection.x === null ||
      (x === selection.x && y === selection.y);

    if (map === selection.map && coordinatesMatch) {
      return index;
    }
  }

  return null;
}

function bindPendingTravelSelection(selector) {
  const selection = pendingTravelSelection;

  if (selection === null || !selection.menuObserved || selection.selector !== null) {
    return;
  }

  try {
    const row = matchingTravelSelectorRow(selector, selection);

    if (row === null) {
      return;
    }

    selection.selector = selector;
    selection.row = row;
  } catch (error) {
    pendingTravelSelection = null;
    send({
      type: 'client-action-failed',
      action: selection.action,
      error: `native selector row binding failed: ${error}`
    });
  }
}

function submitPendingTravelSelection() {
  const selection = pendingTravelSelection;

  if (selection === null ||
      selection.selector === null ||
      selection.row === null ||
      submitTravelSelectorRow === null) {
    return false;
  }

  // Consume before entering native code. This runs only after the network poll
  // that dispatched 0x2e has returned, so the live selector model is complete.
  // The native method reads that row, constructs opcode 0x3f, and enqueues it
  // through NexusTK's ordinary UI-to-network transport queue.
  pendingTravelSelection = null;

  try {
    submitTravelSelectorRow(selection.selector, selection.row);
    return true;
  } catch (error) {
    send({
      type: 'client-action-failed',
      action: selection.action,
      error: `native selector submission failed: ${error}`
    });
    return false;
  }
}

function installTravelSelectorHook() {
  const submit = mainModule.base.add(TRAVEL_SELECTOR_SUBMIT_RVA);
  const submitSignature = [0x55, 0x8b, 0xec, 0x81, 0xec, 0x04, 0x01, 0x00, 0x00];

  if (!matchesBytes(submit, submitSignature)) {
    throw new Error('build-752 travel-selector submit signature mismatch');
  }

  submitTravelSelectorRow = new NativeFunction(
    submit,
    'void',
    ['pointer', 'int32'],
    'thiscall'
  );

  const construct = mainModule.base.add(TRAVEL_SELECTOR_CONSTRUCT_RVA);
  const constructSignature =
    [0x55, 0x8b, 0xec, 0x6a, 0xff, 0x68, 0xae, 0xbf, 0x60, 0x00];

  if (!matchesBytes(construct, constructSignature)) {
    throw new Error('build-752 travel-selector constructor signature mismatch');
  }

  Interceptor.attach(construct, {
    onEnter() {
      this.selector = this.context.ecx;
    },
    onLeave() {
      // The constructor has populated the complete 0x94-byte row vector before
      // returning. Binding here avoids both an unrelated 0x4c-byte container
      // helper and partially initialized selector state. Submit at this same
      // client-owned boundary: the selector is modal, so waiting for another
      // network poll can leave the native 0x3f enqueue permanently uncalled.
      bindPendingTravelSelection(this.selector);
      submitPendingTravelSelection();
    }
  });
}

function beginClientStep(direction) {
  if (clientKeyInfo[direction] === undefined) {
    throw new Error(`unsupported direction ${direction}`);
  }

  if (clientStepState !== null && clientStepState.phase !== 'complete') {
    clientStepState.phase = 'superseded';
  }

  const state = {
    direction,
    phase: 'awaiting-transport-evidence',
    tapAttempts: 0
  };
  clientStepState = state;
  submitClientStepTap(state);
}

function submitClientStepTap(state) {
  if (clientStepState !== state ||
      state.phase === 'complete' ||
      state.phase === 'superseded') {
    return;
  }

  if (state.tapAttempts >= MAX_CLIENT_STEP_TAP_ATTEMPTS) {
    state.phase = 'complete';
    send({
      type: 'warning',
      message: `client step ${state.direction} produced no movement or obstruction after ${state.tapAttempts} taps`
    });
    return;
  }

  state.tapAttempts += 1;

  try {
    invokeClientKeyTap(state.direction);
  } catch (error) {
    state.phase = 'complete';
    send({ type: 'client-action-failed', action: 'step', error: String(error) });
    return;
  }

  // A posted direction can be swallowed while the unfocused client is between
  // input cycles. Keep tapping this one intent until the plaintext hook sees a
  // movement or obstruction body. The hook marks the state complete
  // synchronously, so a successful movement cancels every remaining retry
  // before another key message can be posted.
  setTimeout(() => submitClientStepTap(state), CLIENT_STEP_RETRY_MS);
}

function noteClientStepOutbound(input, length) {
  const state = clientStepState;

  if (state === null || state.phase === 'complete' || state.phase === 'superseded') {
    return;
  }

  try {
    const body = Array.from(new Uint8Array(input.readByteArray(length)));
    const compactMovement = length === 10 && body[0] === 0x32 && body[3] === 0x50;
    const fullMovement = length === 17 && body[0] === 0x06 && body[3] === 0x50;
    const obstruction = length === 7 && body[0] === 0x69;

    if (compactMovement || fullMovement || obstruction) {
      state.phase = 'complete';
      return;
    }

    if (length === 3 && body[0] === 0x11) {
      state.phase = 'turn-observed';
    }
  } catch (error) {
    state.phase = 'complete';
    send({ type: 'client-action-failed', action: 'step', error: String(error) });
  }
}

function invokeClientRefresh() {
  const post = postMessage();
  const hwnd = resolveClientWindow();
  const down = scan => ((scan << 16) | 1) >>> 0;
  const up = scan => (0xc0000001 | (scan << 16)) >>> 0;

  if (post(hwnd, 0x0100, 0x11, down(0x1d)) === 0) {
    throw new Error('PostMessageW(Control down) failed');
  }

  setTimeout(() => {
    try {
      if (post(hwnd, 0x0100, 0x52, down(0x13)) === 0) {
        throw new Error('PostMessageW(R down) failed');
      }

      setTimeout(() => {
        post(hwnd, 0x0101, 0x52, up(0x13));
        post(hwnd, 0x0101, 0x11, up(0x1d));
      }, 60);
    } catch (error) {
      post(hwnd, 0x0101, 0x11, up(0x1d));
      send({ type: 'client-action-failed', action: 'refresh-map', error: String(error) });
    }
  }, 30);
}

function invokeClientCastSpell(slot) {
  const numericSlot = Number(slot);

  if (!Number.isInteger(numericSlot) || numericSlot < 1 || numericSlot > 26) {
    throw new Error(`spell slot must be between 1 and 26 (received ${slot})`);
  }

  if (clientCastState !== null && clientCastState.phase !== 'complete') {
    throw new Error('a client spell cast is already in flight');
  }

  const post = postMessage();
  const hwnd = resolveClientWindow();
  const letterVk = 0x41 + numericSlot - 1;
  const letterScan = clientLetterScanCodes[numericSlot - 1];
  const down = scan => ((scan << 16) | 1) >>> 0;
  const up = scan => (0xc0000001 | (scan << 16)) >>> 0;
  const state = { slot: numericSlot, phase: 'posting-hotkey' };
  clientCastState = state;

  if (post(hwnd, 0x0100, 0x10, down(0x2a)) === 0) {
    state.phase = 'complete';
    throw new Error('PostMessageW(Shift down) failed');
  }

  setTimeout(() => {
    try {
      if (post(hwnd, 0x0100, 0x5a, down(0x2c)) === 0) {
        throw new Error('PostMessageW(Z down) failed');
      }

      setTimeout(() => {
        try {
          post(hwnd, 0x0101, 0x5a, up(0x2c));
          post(hwnd, 0x0101, 0x10, up(0x2a));

          setTimeout(() => {
            try {
              if (post(hwnd, 0x0100, letterVk, down(letterScan)) === 0) {
                throw new Error('PostMessageW(spell letter down) failed');
              }

              setTimeout(() => {
                post(hwnd, 0x0101, letterVk, up(letterScan));

                if (state.phase !== 'complete') {
                  state.phase = 'awaiting-outgoing-cast';
                }
              }, 60);
            } catch (error) {
              state.phase = 'complete';
              send({ type: 'client-action-failed', action: 'cast', error: String(error) });
            }
          }, 90);
        } catch (error) {
          state.phase = 'complete';
          send({ type: 'client-action-failed', action: 'cast', error: String(error) });
        }
      }, 60);
    } catch (error) {
      state.phase = 'complete';
      post(hwnd, 0x0101, 0x10, up(0x2a));
      send({ type: 'client-action-failed', action: 'cast', error: String(error) });
    }
  }, 30);
}

function noteClientCastOutbound(input, length) {
  const state = clientCastState;

  if (state === null || state.phase === 'complete' || length < 2) {
    return;
  }

  if (input.readU8() === 0x0f && input.add(1).readU8() === state.slot) {
    state.phase = 'complete';
  }
}

function forceMapRegionResponse(input, length) {
  if (!forceMapDataEnabled) {
    return;
  }

  try {
    const body = Array.from(new Uint8Array(input.readByteArray(length)));
    let checksumOffset = -1;
    let regionOffset = -1;

    if (length === 11 && body[0] === 0x05) {
      checksumOffset = 7;
      regionOffset = 1;
    } else if (length === 17 && body[0] === 0x06 && body[3] === 0x50) {
      checksumOffset = 14;
      regionOffset = 8;
    }

    if (checksumOffset < 0) {
      return;
    }

    const width = body[regionOffset + 4];
    const height = body[regionOffset + 5];
    const checksum = (body[checksumOffset] << 8) | body[checksumOffset + 1];

    if (width === 0 || height === 0 || width * height > 323 || checksum === 0) {
      return;
    }

    input.add(checksumOffset).writeU8(0);
    input.add(checksumOffset + 1).writeU8(0);
  } catch (error) {
    send({ type: 'warning', message: `map checksum override failed: ${error}` });
  }
}

rpc.exports = {
  clientResources() {
    return readClientResources();
  },
  clientMapContext() {
    return readClientMapContext();
  },
  clientMapStrings(limit) {
    return probeClientMapStrings(limit);
  },
  clientFindText(text) {
    return probeClientText(text);
  },
  clientInventory() {
    return readClientInventory();
  },
  clientSessionReady() {
    return !sessionClosing && outgoingSession !== null && !outgoingSession.isNull();
  },
  clientStep(direction) {
    beginClientStep(String(direction));
    return true;
  },
  clientFace(direction) {
    return invokeClientFace(direction);
  },
  clientRefresh() {
    invokeClientRefresh();
    return true;
  },
  clientActivity() {
    invokeClientActivity();
    return true;
  },
  clientDismissOverlay() {
    invokeVirtualKeyTap(0x1b, 0x01, 'dismiss-overlay');
    return true;
  },
  clientRequestProfile() {
    // The body is the captured normal self-profile request. It is submitted
    // through the client's already-live crypto/session routine, not a raw
    // socket, so nonce and transport state remain owned by NexusTK.
    return invokePlaintextBody([0x2d, 0x00, 0x00], 'request-profile');
  },
  clientCastSpell(slot) {
    invokeClientCastSpell(slot);
    return true;
  },
  clientAttack(direction) {
    return invokeClientAttack(direction);
  },
  clientPickup() {
    return invokeClientPickup();
  },
  clientUseInventory(slot) {
    invokeClientUseInventory(slot);
    return true;
  },
  clientInteract(entity) {
    return invokeClientInteract(entity);
  },
  clientDialog(entity, command, argument, quantity) {
    return invokeClientDialog(entity, command, argument, quantity);
  },
  clientDialogTransaction(entity, selections) {
    return invokeClientDialogTransaction(entity, selections);
  },
  clientSpeak(channel, text) {
    return invokeClientSpeak(channel, text);
  },
  clientAnsweredSpell(slot, answer) {
    return invokeClientAnsweredSpell(slot, answer);
  },
  clientTravel(map) {
    return invokeClientTravel(map);
  },
  clientTravelOnMenu(map) {
    stageClientTravelOnMenu(map);
    return true;
  },
  setForceMapData(enabled) {
    forceMapDataEnabled = Boolean(enabled);
    return forceMapDataEnabled === Boolean(enabled);
  }
};

const outgoingAddress = resolveHook('cryptOutgoingPacket', CONFIG.outgoingRva);
const incomingAddress = resolveHook('cryptIncomingPacket', CONFIG.incomingRva);
const networkPollAddress = resolveHook('networkPoll', 0x001786f0);
const networkPollSignature = [0x55, 0x8b, 0xec, 0x6a, 0xff, 0x68];

if (!matchesBytes(networkPollAddress, networkPollSignature)) {
  throw new Error('build-752 network-poll signature mismatch');
}

const cryptAndSend = new NativeFunction(
  outgoingAddress,
  'void',
  ['pointer', 'pointer', 'uint16'],
  'thiscall'
);

Interceptor.attach(networkPollAddress, {
  onEnter() {
    this.session = this.context.ecx;
  },
  onLeave() {
    flushPlaintextBatch(this.session, this.threadId);
    submitPendingTravelSelection();
  }
});

Interceptor.attach(outgoingAddress, {
  onEnter(args) {
    if (directPlaintextSendDepth !== 0) {
      this.directPlaintext = true;
      return;
    }

    this.session = this.context.ecx;
    const input = args[0];
    const length = args[1].toUInt32() & 0xffff;
    const opcode = length > 0 && !input.isNull() ? input.readU8() : null;

    // A clean 0x0B invalidates the captured session object immediately. Only
    // a later login request can establish another sender in the same process.
    if (opcode === 0x03) {
      sessionClosing = false;
      failAllPendingPlaintextBatches('a new login session replaced the queued client session');
      pendingTravelSelection = null;
    }
    if (!sessionClosing) {
      outgoingSession = this.context.ecx;
    }
    satisfyPendingNativeBody(opcode);
    noteClientStepOutbound(input, length);
    noteClientCastOutbound(input, length);
    noteClientInventoryOutbound(input, length);
    forceMapRegionResponse(input, length);
    emitPacket('outgoing', input, length, this.threadId);
    if (opcode === 0x0b) {
      sessionClosing = true;
      outgoingSession = null;
      failAllPendingPlaintextBatches('the client session closed before dispatch');
      pendingTravelSelection = null;
    }
  },
  onLeave() {
    if (this.directPlaintext) {
      return;
    }

    // A real client outbound body (including the obstruction emitted by a
    // directional key pressed into a tree) proves that this is the serialized
    // game-network thread. Drain the first queued logical transaction only
    // after that body has completed, so the client's session counter and
    // cipher remain the sole source of ordering. Re-entry from cryptAndSend
    // sees the direct-send guard.
    flushPlaintextBatch(this.session, this.threadId);
  }
});

Interceptor.attach(incomingAddress, {
  onEnter(args) {
    // In build 752 the incoming and outgoing crypto methods are members of
    // the same network/session object. Any successfully dispatched incoming
    // body therefore establishes the exact `this` pointer needed by direct
    // client-authored actions, even when a warm attachment has not yet seen
    // an outgoing packet.
    const session = this.context.ecx;

    if (!session.isNull()) {
      outgoingSession = session;
      sessionClosing = false;
    }

    this.session = session;
    this.length = args[1].toUInt32();
    this.output = args[2];
  },
  onLeave() {
    emitPacket('incoming', this.output, this.length, this.threadId);
    // This boundary returns to the caller that still has to dispatch the
    // decrypted body. It may observe the menu, but must never re-enter the
    // outgoing cipher or mutate its sequence state.
    observePendingTravelMenu(this.output, this.length);
    observeDialogResponse(this.output, this.length);
  }
});

installTravelSelectorHook();
installTransportHooks();

send({
  type: 'ready',
  pid: Process.id,
  arch: Process.arch,
  module: mainModule.name,
  outgoing: outgoingAddress.toString(),
  incoming: incomingAddress.toString()
});
