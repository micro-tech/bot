import * as hello from './hello';
import * as ping from './ping';
import * as help from './help';
import { Message } from 'discord.js';

const commandMap = new Map<string, (message: Message) => Promise<void>>();
commandMap.set(hello.name, hello.execute);
commandMap.set(ping.name, ping.execute);
commandMap.set(help.name, help.execute);

export { commandMap };