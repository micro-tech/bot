import { Client } from 'discord.js';
import logger from '../utils/logger';

export const execute = (readyClient: Client) => {
  logger.info(`Ready! Logged in as ${readyClient.user.tag}`);
};