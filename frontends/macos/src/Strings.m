#pragma once

#import <Foundation/Foundation.h>

// Looks up a key in Localizable.strings (en, et).
static NSString *localized(NSString *key)
{
    return [[NSBundle mainBundle] localizedStringForKey:key value:nil table:nil];
}
