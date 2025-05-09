import { mock } from 'node:test';
import type { Protocol } from 'playwright-core/types/protocol';
import { WebSocketServer } from 'ws';

export interface BrowserServerMockParams {
  logReceivedMessages?: boolean;
}

export interface IncomingMessage {
  id: number;
  method: string;
  params: unknown;
  sessionId?: string;
}
export function createBrowserServerMock(params: BrowserServerMockParams = {}) {
  const port = 8080;

  const browserContextId = '90528612930E8EA3DC51FA91C65089F4';
  const targetId = '05A0C2CA21E1C98F8AE96AF94CF58A0A';
  const loaderId = '04A0197ED46C80B4BA0592BC4B8172D7';
  const sessionId = 'FC97DB2DBA44FB5446891A8A74057F33';
  const windowId = 61956135;
  const utilityScriptObjectId = '2533827586503800783.4.1';

  const getOnSendCallback = (context: string) => {
    return (err: Error | null | undefined) => {
      if (err) {
        console.error(`error (${context}): %s`, err);
      }
    };
  };

  const messages: Array<IncomingMessage> = [];
  const runtimeCallFunctionOn = mock.fn<
    (params: Protocol.CommandParameters['Runtime.callFunctionOn']) => Protocol.Runtime.RemoteObject
  >(() => ({ type: 'string', value: 'Hello from Retrack.dev!' }));
  const wss = new WebSocketServer({ port });
  let scriptIdentifier = 0;
  wss.on('connection', function connection(ws) {
    ws.on('error', getOnSendCallback('error-event'));

    let executionContextIdCounter = 0;
    ws.on('message', function message(rawMessage: ArrayBuffer) {
      if (params.logReceivedMessages) {
        console.debug(`received: %s`, rawMessage);
      }

      const message = JSON.parse(rawMessage.toString()) as IncomingMessage;
      messages.push(message);

      // Ceremony: browser initialization
      // pw:protocol SEND ► {"id":1,"method":"Browser.getVersion"}
      // pw:protocol ◀ RECV {"id":1,"result":{"protocolVersion":"1.3","product":"Chrome/128.0.6613.113","revision":"@9597ae93a15d4d03089b4e9997b1072228baa9ad","userAgent":"HeadlessChrome/128.0.0.0","jsVersion":"12.8.374.24"}}
      if (message.method === 'Browser.getVersion') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: {
              protocolVersion: '1.3',
              product: 'Chrome/136.0.7103.93',
              revision: '@d15f7e0b7b458c1502136e8aee33a8187c49a489',
              userAgent: 'HeadlessChrome/136.0.0.0',
              jsVersion: '13.6.233.8',
            },
          }),
          getOnSendCallback('Browser.getVersion'),
        );
      }

      // Ceremony: new browser context
      //   pw:protocol SEND ► {"id":3,"method":"Target.createBrowserContext","params":{"disposeOnDetach":true}}
      //   pw:protocol ◀ RECV {"id":3,"result":{"browserContextId":"90528612930E8EA3DC51FA91C65089F4"}}
      if (message.method === 'Target.createBrowserContext') {
        return ws.send(
          JSON.stringify({ id: message.id, result: { browserContextId } }),
          getOnSendCallback('Target.createBrowserContext'),
        );
      }

      // Ceremony: new target (page)
      //   pw:protocol SEND ► {"id":5,"method":"Target.createTarget","params":{"url":"about:blank","browserContextId":"90528612930E8EA3DC51FA91C65089F4"}}
      //   pw:protocol ◀ RECV {"id":5,"result":{"targetId":"05A0C2CA21E1C98F8AE96AF94CF58A0A"}} +6ms
      if (message.method === 'Target.createTarget') {
        // Because of this this
        //   pw:protocol SEND ► {"id":2,"method":"Target.setAutoAttach","params":{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}}
        //   pw:protocol ◀ RECV {"id":2,"result":{}} +1ms

        //   pw:protocol ◀ RECV {"method":"Target.attachedToTarget","params":{"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43","targetInfo":{"targetId":"B3A3B434CC5001702EA73A7B76F02DB4","type":"page","title":"","url":"about:blank","attached":true,"canAccessOpener":false,"browserContextId":"38E5A7C26517FD366108C7A312C21FF1"},"waitingForDebugger":true}}
        ws.send(
          JSON.stringify({
            method: 'Target.attachedToTarget',
            params: {
              sessionId,
              targetInfo: {
                targetId,
                type: 'page',
                title: 'New Tab',
                url: 'about:blank',
                attached: true,
                canAccessOpener: false,
                browserContextId,
              },
            },
            waitingForDebugger: true,
          }),
          getOnSendCallback('Target.attachedToTarget'),
        );

        return ws.send(
          JSON.stringify({ id: message.id, result: { targetId } }),
          getOnSendCallback('Target.createTarget'),
        );
      }

      //   pw:protocol SEND ► {"id":6,"method":"Browser.getWindowForTarget","sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"}
      //   pw:protocol ◀ RECV {"id":6,"result":{"windowId":395781215,"bounds":{"left":22,"top":47,"width":1200,"height":1371,"windowState":"normal"}},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"}
      if (message.method === 'Browser.getWindowForTarget') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: { windowId, bounds: { left: 10, top: 100, width: 1200, height: 1368, windowState: 'normal' } },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Browser.getWindowForTarget'),
        );
      }

      //   pw:protocol SEND ► {"id":8,"method":"Page.getFrameTree","sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"}
      //   pw:protocol ◀ RECV {"id":8,"result":{"frameTree":{"frame":{"id":"B3A3B434CC5001702EA73A7B76F02DB4","loaderId":"DA0E26D2A15233E2E16AFEDB6F4654BC","url":"about:blank","domainAndRegistry":"","securityOrigin":"://","mimeType":"text/html","adFrameStatus":{"adFrameType":"none"},"secureContextType":"InsecureScheme","crossOriginIsolatedContextType":"NotIsolated","gatedAPIFeatures":[]}}
      if (message.method === 'Page.getFrameTree') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: {
              frameTree: {
                frame: {
                  id: targetId,
                  loaderId,
                  url: 'about:blank',
                  domainAndRegistry: '',
                  securityOrigin: '://',
                  mimeType: 'text/html',
                  adFrameStatus: { adFrameType: 'none' },
                  secureContextType: 'InsecureScheme',
                  crossOriginIsolatedContextType: 'NotIsolated',
                  gatedAPIFeatures: [],
                },
              },
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.getFrameTree'),
        );
      }

      //   pw:protocol SEND ► {"id":11,"method":"Runtime.enable","params":{},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +0ms
      //   pw:protocol ◀ RECV {"id":11,"result":{},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +0ms
      if (message.method === 'Runtime.enable') {
        // Automatically generated when runtime is enabled
        //   pw:protocol ◀ RECV {"method":"Runtime.executionContextCreated","params":{"context":{"id":1,"origin":"://","name":"","uniqueId":"-4794704493778289939.5863022730874866662","auxData":{"isDefault":true,"type":"default","frameId":"B3A3B434CC5001702EA73A7B76F02DB4"}}},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"}
        ws.send(
          JSON.stringify({
            method: 'Runtime.executionContextCreated',
            params: {
              context: {
                id: ++executionContextIdCounter,
                origin: '://',
                name: '',
                uniqueId: '-2598416221269343120.6898192793065449982',
                auxData: { isDefault: true, type: 'default', frameId: targetId },
              },
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Runtime.executionContextCreated'),
        );
        return ws.send(
          JSON.stringify({ id: message.id, result: {}, sessionId: message.sessionId }),
          getOnSendCallback('Runtime.enable'),
        );
      }

      //  pw:protocol SEND ► {"id":21,"method":"Page.createIsolatedWorld","params":{"frameId":"B3A3B434CC5001702EA73A7B76F02DB4","grantUniveralAccess":true,"worldName":"__playwright_utility_world__"},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +1ms
      //  pw:protocol ◀ RECV {"id":21,"result":{"executionContextId":2},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +0ms
      if (message.method === 'Page.createIsolatedWorld') {
        const worldParams = message.params as { frameId: string; worldName: string };

        // New isolated word uses its own execution context.
        //   pw:protocol ◀ RECV {"method":"Runtime.executionContextCreated","params":{"context":{"id":2,"origin":"","name":"__playwright_utility_world__","uniqueId":"3619076036493720473.2291309561558851914","auxData":{"isDefault":false,"type":"isolated","frameId":"B3A3B434CC5001702EA73A7B76F02DB4"}}},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +1ms
        const worldContextId = ++executionContextIdCounter;
        ws.send(
          JSON.stringify({
            method: 'Runtime.executionContextCreated',
            params: {
              context: {
                id: worldContextId,
                origin: '',
                name: worldParams.worldName,
                uniqueId: `-5958853809744475408.-604${worldContextId}01954874867067${worldContextId}`,
                auxData: { isDefault: false, type: 'isolated', frameId: worldParams.frameId },
              },
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Runtime.executionContextCreated via Page.createIsolatedWorld'),
        );
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: { executionContextId: worldContextId },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.createIsolatedWorld'),
        );
      }

      //   pw:protocol SEND ► {"id":12,"method":"Page.addScriptToEvaluateOnNewDocument","params":{"source":"","worldName":"__playwright_utility_world__"},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +0ms
      //   pw:protocol ◀ RECV {"id":12,"result":{"identifier":"1"},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +0ms
      if (message.method === 'Page.addScriptToEvaluateOnNewDocument') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: { identifier: `${++scriptIdentifier}` },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.addScriptToEvaluateOnNewDocument'),
        );
      }

      // Ceremony: page navigation
      //   pw:protocol SEND ► {"id":22,"method":"Page.navigate","params":{"url":"https://y5l3m6u-demo.webhooks.dev.secutils.dev/","frameId":"47AAACCFCAA5C17FD8E42179CD506CBD","referrerPolicy":"unsafeUrl"},"sessionId":"3F2683741E9935900DDAB6B76CC96699"}
      //   pw:protocol ◀ RECV {"id":22,"result":{"frameId":"47AAACCFCAA5C17FD8E42179CD506CBD","loaderId":"93512EFAE4DED1EAA9180540339BB64A"},"sessionId":"3F2683741E9935900DDAB6B76CC96699"} +2ms
      if (message.method === 'Page.navigate') {
        const messageParams = message.params as { frameId: string };
        ws.send(
          JSON.stringify({
            id: message.id,
            result: { frameId: messageParams.frameId, loaderId },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.navigate'),
        );

        //   pw:protocol ◀ RECV {"method":"Page.frameNavigated","params":{"frame":{"id":"47AAACCFCAA5C17FD8E42179CD506CBD","loaderId":"93512EFAE4DED1EAA9180540339BB64A","url":"https://y5l3m6u-demo.webhooks.dev.secutils.dev/","domainAndRegistry":"secutils.dev","securityOrigin":"https://y5l3m6u-demo.webhooks.dev.secutils.dev","mimeType":"text/html","adFrameStatus":{"adFrameType":"none"},"secureContextType":"Secure","crossOriginIsolatedContextType":"NotIsolated","gatedAPIFeatures":[]},"type":"Navigation"},"sessionId":"3F2683741E9935900DDAB6B76CC96699"} +1ms
        ws.send(
          JSON.stringify({
            method: 'Page.frameNavigated',
            params: {
              frame: {
                id: messageParams.frameId,
                loaderId,
                url: 'https://retrack.dev',
                domainAndRegistry: 'retrack.dev',
                securityOrigin: 'https://retrack.dev',
                mimeType: 'text/html',
                adFrameStatus: { adFrameType: 'none' },
                secureContextType: 'Secure',
                crossOriginIsolatedContextType: 'NotIsolated',
                gatedAPIFeatures: [],
              },
              type: 'Navigation',
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.frameNavigated'),
        );

        //   pw:protocol ◀ RECV {"method":"Page.loadEventFired","params":{"timestamp":470419.830279},"sessionId":"3F2683741E9935900DDAB6B76CC96699"} +0ms
        //   pw:protocol ◀ RECV {"method":"Page.lifecycleEvent","params":{"frameId":"47AAACCFCAA5C17FD8E42179CD506CBD","loaderId":"93512EFAE4DED1EAA9180540339BB64A","name":"load","timestamp":470419.830279},"sessionId":"3F2683741E9935900DDAB6B76CC96699"} +0ms
        return ws.send(
          JSON.stringify({
            method: 'Page.lifecycleEvent',
            params: { frameId: messageParams.frameId, loaderId, name: 'load', timestamp: 470419.830279 },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Page.lifecycleEvent (load)'),
        );
      }

      // Ceremony: evaluate (Playwright utility script)
      //   pw:protocol SEND ► {"id":23,"method":"Runtime.evaluate","params":{"expression":"...return new (module.exports.UtilityScript())(false);","contextId":4},"sessionId":"64808CB073F6E16A7E6006F5FD7C9A7D"} +4s
      //   pw:protocol ◀ RECV {"id":23,"result":{"result":{"type":"object","className":"UtilityScript","description":"UtilityScript","objectId":"2533827586503800783.4.1"}},"sessionId":"64808CB073F6E16A7E6006F5FD7C9A7D"} +2ms
      if (message.method === 'Runtime.evaluate') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: {
              result: (message.params as { expression: string }).expression.includes('CustomEvent')
                ? { type: 'boolean', value: true }
                : {
                    type: 'object',
                    className: 'UtilityScript',
                    description: 'UtilityScript',
                    objectId: utilityScriptObjectId,
                  },
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Runtime.evaluate (UtilityScript)'),
        );
      }

      //   pw:protocol SEND ► {"id":24,"method":"Runtime.callFunctionOn","params":{"functionDeclaration":"(utilityScript, ...args) => utilityScript.evaluate(...args)","objectId":"2533827586503800783.4.1","arguments":[{"objectId":"2533827586503800783.4.1"},{"value":true},{"value":true},{"value":"() => {\n        let retVal = '';\n        if (document.doctype) retVal = new XMLSerializer().serializeToString(document.doctype);\n        if (document.documentElement) retVal += document.documentElement.outerHTML;\n        return retVal;\n      }"},{"value":1},{"value":{"v":"undefined"}}],"returnByValue":true,"awaitPromise":true,"userGesture":true},"sessionId":"64808CB073F6E16A7E6006F5FD7C9A7D"} +0ms
      //   pw:protocol ◀ RECV {"id":24,"result":{"result":{"type":"string","value":"<html><head></head><body>Hello from <a href=\"https://secutils.dev\">Secutils.dev</a>!</body></html>"}},"sessionId":"64808CB073F6E16A7E6006F5FD7C9A7D"} +1ms
      if (message.method === 'Runtime.callFunctionOn') {
        return ws.send(
          JSON.stringify({
            id: message.id,
            result: {
              result: runtimeCallFunctionOn(message.params as Protocol.CommandParameters['Runtime.callFunctionOn']),
            },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Runtime.callFunctionOn'),
        );
      }

      // Ceremony: page close
      //   pw:protocol SEND ► {"id":25,"method":"Target.closeTarget","params":{"targetId":"B3A3B434CC5001702EA73A7B76F02DB4"}} +977ms
      //   pw:protocol ◀ RECV {"id":25,"result":{"success":true}} +1ms
      if (message.method === 'Target.closeTarget') {
        const messageParams = message.params as { targetId: string; sessionId?: string };
        ws.send(JSON.stringify({ id: message.id, result: { success: true } }), getOnSendCallback('Page.navigate'));

        //   pw:protocol ◀ RECV {"method":"Inspector.detached","params":{"reason":"Render process gone."},"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43"} +1ms
        //   pw:protocol ◀ RECV {"method":"Target.detachedFromTarget","params":{"sessionId":"AF1BE27C8937BCB6754D2280EC8DDD43","targetId":"B3A3B434CC5001702EA73A7B76F02DB4"}} +1ms
        return ws.send(
          JSON.stringify({
            method: 'Target.detachedFromTarget',
            params: { targetId: messageParams.targetId, sessionId: messageParams.sessionId ?? sessionId },
            sessionId: message.sessionId,
          }),
          getOnSendCallback('Target.detachedFromTarget'),
        );
      }

      ws.send(
        JSON.stringify({ id: message.id, result: {}, sessionId: message.sessionId }),
        getOnSendCallback(message.method),
      );
    });
  });

  return {
    endpoint: `ws://localhost:${port}`,
    messages,
    runtimeCallFunctionOn,
    isBuiltInPageContent: (params: Protocol.CommandParameters['Runtime.callFunctionOn']) =>
      params.objectId === utilityScriptObjectId &&
      params.arguments?.some((arg) => typeof arg.value === 'string' && arg.value.includes('serializeToString')),
    cleanup: () => new Promise((resolve, reject) => wss.close((err) => (err ? reject(err) : resolve(undefined)))),
  };
}
