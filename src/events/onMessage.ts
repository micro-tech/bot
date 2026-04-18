import { Events, Message } from 'discord.js';
import { commandMap } from '../commands';
import logger from '../utils/logger';

export const name = Events.MessageCreate;
export const execute = async (message: Message) => {
  if (message.author.bot) return;
  try {
    const content = message.content.toLowerCase();
    const command = commandMap.get(content);
    if (command) {
      await command(message);
    }
  } catch (error) {
    logger.error(`Error handling message: ${error}`);
  }
};