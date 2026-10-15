import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const IFACE = 'org.gnome.Shell.Extensions.WayScrollShotHelper';
const OBJECT_PATH = '/org/gnome/Shell/Extensions/WayScrollShotHelper';
const API_VERSION = '1';

const DBUS_XML = `
<node>
  <interface name="${IFACE}">
    <method name="Version">
      <arg type="s" name="version" direction="out"/>
    </method>
    <method name="SelectArea">
      <arg type="b" name="success" direction="out"/>
      <arg type="i" name="x" direction="out"/>
      <arg type="i" name="y" direction="out"/>
      <arg type="i" name="width" direction="out"/>
      <arg type="i" name="height" direction="out"/>
    </method>
    <method name="ScreenshotArea">
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
      <arg type="i" name="width" direction="in"/>
      <arg type="i" name="height" direction="in"/>
      <arg type="s" name="filename" direction="in"/>
      <arg type="b" name="success" direction="out"/>
      <arg type="s" name="filename_used" direction="out"/>
    </method>
    <method name="ShowControls">
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
      <arg type="i" name="width" direction="in"/>
      <arg type="i" name="height" direction="in"/>
    </method>
    <method name="WaitAction">
      <arg type="s" name="action" direction="out"/>
    </method>
    <method name="HideControls"/>
  </interface>
</node>`;

function eventPoint(event) {
    if (event?.get_coords) {
        const coords = event.get_coords();
        if (Array.isArray(coords) && coords.length >= 3 && typeof coords[1] === 'number') {
            return [coords[1], coords[2]];
        }
        if (Array.isArray(coords) && coords.length >= 2 && typeof coords[0] === 'number') {
            return [coords[0], coords[1]];
        }
    }
    if (global.get_pointer) {
        const pointer = global.get_pointer();
        return [pointer[0], pointer[1]];
    }
    return [0, 0];
}

function finishScreenshotArea(source, result, stream, filename, invocation) {
    try {
        const finishResult = source.screenshot_area_finish(result);
        const success = Array.isArray(finishResult) ? finishResult[0] : Boolean(finishResult);
        const returnedPath = Array.isArray(finishResult) && typeof finishResult[1] === 'string'
            ? finishResult[1]
            : '';
        invocation.return_value(new GLib.Variant('(bs)', [success, stream ? filename : returnedPath || filename]));
    } catch (e) {
        invocation.return_dbus_error(`${IFACE}.Error`, e.message);
    } finally {
        if (stream) {
            try {
                stream.close(null);
            } catch (closeError) {
                console.log(`[wayscrollshot] failed to close screenshot stream: ${closeError.message}`);
            }
        }
    }
}

export default class WayScrollShotHelper extends Extension {
    enable() {
        this._screenshot = new Shell.Screenshot();
        this._selection = null;
        this._controls = null;
        this._waitInvocation = null;
        this._pendingAction = null;
        const interfaceInfo = Gio.DBusNodeInfo.new_for_xml(DBUS_XML).interfaces[0];
        this._dbusId = Gio.DBus.session.register_object(
            OBJECT_PATH,
            interfaceInfo,
            (_connection, _sender, _objectPath, _interfaceName, methodName, parameters, invocation) => {
                this._handleMethod(methodName, parameters, invocation);
            },
            null,
            null,
        );
        console.log('[wayscrollshot] scroll helper enabled');
    }

    disable() {
        this._cancelSelection(false);
        this._hideControls();
        if (this._waitInvocation) {
            this._waitInvocation.return_dbus_error(`${IFACE}.Error`, 'extension disabled');
            this._waitInvocation = null;
        }
        if (this._dbusId) {
            Gio.DBus.session.unregister_object(this._dbusId);
            this._dbusId = 0;
        }
        this._screenshot = null;
    }

    _handleMethod(methodName, parameters, invocation) {
        try {
            if (methodName === 'Version') {
                invocation.return_value(new GLib.Variant('(s)', [API_VERSION]));
                return;
            }
            if (methodName === 'SelectArea') {
                this._beginSelection(invocation);
                return;
            }
            if (methodName === 'ScreenshotArea') {
                this._screenshotArea(parameters, invocation);
                return;
            }
            if (methodName === 'ShowControls') {
                const [x, y, width, height] = parameters.deepUnpack();
                this._showControls(x, y, width, height);
                invocation.return_value(null);
                return;
            }
            if (methodName === 'WaitAction') {
                this._waitAction(invocation);
                return;
            }
            if (methodName === 'HideControls') {
                this._hideControls();
                if (this._waitInvocation) {
                    const waiting = this._waitInvocation;
                    this._waitInvocation = null;
                    waiting.return_value(new GLib.Variant('(s)', ['cancel']));
                }
                invocation.return_value(null);
                return;
            }
            invocation.return_dbus_error(`${IFACE}.Error`, `unknown method: ${methodName}`);
        } catch (e) {
            invocation.return_dbus_error(`${IFACE}.Error`, e.message);
        }
    }

    _beginSelection(invocation) {
        if (this._selection) {
            invocation.return_dbus_error(`${IFACE}.Error`, 'selection already in progress');
            return;
        }
        const overlay = new St.Widget({
            reactive: true,
            can_focus: true,
            x: 0,
            y: 0,
            width: global.stage.width,
            height: global.stage.height,
            style: 'background-color: rgba(0, 0, 0, 0.12);',
        });
        const rubber = new St.Widget({
            visible: false,
            style: 'border: 2px solid rgba(76, 154, 255, 0.95); background-color: rgba(76, 154, 255, 0.18);',
        });
        overlay.add_child(rubber);
        Main.layoutManager.addChrome(overlay);
        const grab = Main.pushModal(overlay, {actionMode: Shell.ActionMode.NORMAL});
        const handler = global.stage.connect('captured-event', (_actor, event) => {
            this._onSelectionEvent(event);
            return Clutter.EVENT_STOP;
        });
        this._selection = {invocation, overlay, rubber, grab, handler, origin: null};
        overlay.grab_key_focus();
    }

    _onSelectionEvent(event) {
        const selection = this._selection;
        if (!selection) {
            return;
        }
        const type = event.type();
        if (type === Clutter.EventType.KEY_PRESS || type === Clutter.EventType.KEY_RELEASE) {
            const symbol = event.get_key_symbol?.();
            if (symbol === Clutter.KEY_Escape || symbol === 65307) {
                this._cancelSelection(false);
            }
            return;
        }
        const [x, y] = eventPoint(event);
        if (type === Clutter.EventType.BUTTON_PRESS) {
            selection.origin = [x, y];
            selection.rubber.visible = true;
            this._placeRubber(x, y, x, y);
            return;
        }
        if (!selection.origin) {
            return;
        }
        if (type === Clutter.EventType.MOTION) {
            this._placeRubber(selection.origin[0], selection.origin[1], x, y);
            return;
        }
        if (type === Clutter.EventType.BUTTON_RELEASE) {
            const rect = this._normalize(selection.origin[0], selection.origin[1], x, y);
            const invocation = selection.invocation;
            this._destroySelection();
            if (rect.width < 2 || rect.height < 2) {
                invocation.return_value(new GLib.Variant('(biiii)', [false, 0, 0, 0, 0]));
                return;
            }
            invocation.return_value(new GLib.Variant('(biiii)', [
                true,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            ]));
        }
    }

    _placeRubber(x1, y1, x2, y2) {
        const rect = this._normalize(x1, y1, x2, y2);
        const rubber = this._selection?.rubber;
        if (!rubber) {
            return;
        }
        rubber.set_position(rect.x, rect.y);
        rubber.set_size(Math.max(rect.width, 1), Math.max(rect.height, 1));
    }

    _normalize(x1, y1, x2, y2) {
        const x = Math.round(Math.min(x1, x2));
        const y = Math.round(Math.min(y1, y2));
        return {
            x,
            y,
            width: Math.round(Math.abs(x2 - x1)),
            height: Math.round(Math.abs(y2 - y1)),
        };
    }

    _cancelSelection(success) {
        const selection = this._selection;
        if (!selection) {
            return;
        }
        const invocation = selection.invocation;
        this._destroySelection();
        invocation.return_value(new GLib.Variant('(biiii)', [success, 0, 0, 0, 0]));
    }

    _destroySelection() {
        const selection = this._selection;
        if (!selection) {
            return;
        }
        this._selection = null;
        if (selection.handler) {
            global.stage.disconnect(selection.handler);
        }
        if (selection.grab) {
            Main.popModal(selection.grab);
        }
        selection.overlay.destroy();
    }

    _screenshotArea(parameters, invocation) {
        const screenshot = global.screenshot ?? this._screenshot;
        if (!screenshot?.screenshot_area) {
            invocation.return_dbus_error(`${IFACE}.Error`, 'GNOME Shell screenshot API is unavailable');
            return;
        }
        const [x, y, width, height, filename] = parameters.deepUnpack();
        let stream = null;
        try {
            const file = Gio.File.new_for_path(filename);
            stream = file.replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
            screenshot.screenshot_area(x, y, width, height, stream, (source, result) => {
                finishScreenshotArea(source, result, stream, filename, invocation);
            });
        } catch (e) {
            if (stream) {
                try {
                    stream.close(null);
                } catch (closeError) {
                    console.log(`[wayscrollshot] failed to close screenshot stream: ${closeError.message}`);
                }
            }
            try {
                screenshot.screenshot_area(x, y, width, height, filename, (source, result) => {
                    finishScreenshotArea(source, result, null, filename, invocation);
                });
            } catch (legacyError) {
                invocation.return_dbus_error(`${IFACE}.Error`, legacyError.message || e.message);
            }
        }
    }

    _showControls(x, y, width, height) {
        this._hideControls();
        const panel = new St.BoxLayout({
            orientation: Clutter.Orientation.HORIZONTAL,
            style: 'background-color: rgba(22, 22, 24, 0.94); border-radius: 10px; padding: 8px;',
            reactive: true,
        });
        for (const [label, action] of [
            ['Save', 'save'],
            ['Copy', 'copy'],
            ['Pause', 'pause'],
            ['Cancel', 'cancel'],
        ]) {
            const button = new St.Button({
                label,
                style: 'padding: 6px 12px; margin: 0 4px; border-radius: 6px; background-color: rgba(255, 255, 255, 0.08); color: white;',
            });
            button.connect('clicked', () => this._finishAction(action));
            panel.add_child(button);
        }
        Main.layoutManager.addChrome(panel);
        const monitor = this._monitorFor(x, y);
        const panelWidth = 280;
        const panelHeight = 48;
        let px = x + width + 12;
        let py = y;
        if (px + panelWidth > monitor.x + monitor.width) {
            px = x;
            py = y + height + 12;
        }
        if (py + panelHeight > monitor.y + monitor.height) {
            py = Math.max(monitor.y, y - panelHeight - 12);
        }
        panel.set_position(px, py);
        this._controls = panel;
    }

    _monitorFor(x, y) {
        const monitors = Main.layoutManager.monitors ?? [];
        for (const monitor of monitors) {
            if (x >= monitor.x && y >= monitor.y && x < monitor.x + monitor.width && y < monitor.y + monitor.height) {
                return monitor;
            }
        }
        return Main.layoutManager.primaryMonitor ?? {x: 0, y: 0, width: global.stage.width, height: global.stage.height};
    }

    _hideControls() {
        if (this._controls) {
            this._controls.destroy();
            this._controls = null;
        }
    }

    _waitAction(invocation) {
        if (this._pendingAction) {
            const action = this._pendingAction;
            this._pendingAction = null;
            invocation.return_value(new GLib.Variant('(s)', [action]));
            return;
        }
        if (this._waitInvocation) {
            this._waitInvocation.return_dbus_error(`${IFACE}.Error`, 'replaced by a newer wait');
        }
        this._waitInvocation = invocation;
    }

    _finishAction(action) {
        if (this._waitInvocation) {
            const invocation = this._waitInvocation;
            this._waitInvocation = null;
            invocation.return_value(new GLib.Variant('(s)', [action]));
            return;
        }
        this._pendingAction = action;
    }
}
